//! TypeSafe's Jev: a decision model. `POST {base}/v1/systemone` with a
//! Bearer key and `{model, state, questions}`; the reply is `answers` (a
//! `noul`'s probability under `noul`) and `usage`. OpenRouter serves Jev
//! models at `{base}/systemone`. No Rust crate speaks it, Rig included.

use std::collections::BTreeMap;

use oxplow_ai::capabilities::CapabilitySupport;
use oxplow_ai::client::{
    bearer, parse_answers, AiError, Answer, CompleteRequest, Completion, DecideRequest, Decision,
    Http, ModelProvider, ProviderInstance, Question,
};
use serde_json::{json, Map, Value};

/// Jev's questions: a `choice` question's criteria are `{option: null}`, a
/// `score` question's its ordered levels; a `noul` has none.
fn questions(questions: &BTreeMap<String, Question>) -> Value {
    let out: Map<String, Value> = questions
        .iter()
        .map(|(name, q)| {
            let q = match q {
                Question::Noul { instructions } => {
                    json!({ "type": "noul", "instructions": instructions })
                }
                Question::Choice {
                    instructions,
                    options,
                } => json!({ "type": "choice", "instructions": instructions,
                    "criteria": options.iter().map(|o| (o.clone(), Value::Null)).collect::<Map<_, _>>() }),
                Question::Score {
                    instructions,
                    levels,
                } => json!({ "type": "score", "instructions": instructions, "criteria": levels }),
            };
            (name.clone(), q)
        })
        .collect();
    Value::Object(out)
}

/// Ask Jev `req` at `url`.
pub async fn decide(
    http: &Http,
    instance: &ProviderInstance<'_>,
    url: &str,
    req: &DecideRequest<'_>,
) -> Result<Decision, AiError> {
    let body =
        json!({ "model": req.model, "state": req.state, "questions": questions(req.questions) });
    let reply = http
        .post(instance.id, url, bearer(instance.key), body)
        .await
        .map_err(|e| redacted(e, instance.key))?;
    let answers = parse_answers(instance.id, &reply["answers"], req.questions, "noul")?;
    let count = |key: &str| reply["usage"][key].as_i64().unwrap_or(0);
    Ok(Decision {
        answers,
        input_tokens: count("input_tokens"),
        output_tokens: count("output_tokens"),
    })
}

/// `error` with the key taken out of an echoed body.
fn redacted(error: AiError, key: Option<&str>) -> AiError {
    match error {
        AiError::Http {
            provider,
            status,
            body,
        } => AiError::Http {
            provider,
            status,
            body: crate::errors::redact(&body, key),
        },
        other => other,
    }
}

/// A connection check that asks one yes/no question.
pub async fn test_by_deciding<P: ModelProvider + ?Sized>(
    provider: &P,
    instance: &ProviderInstance<'_>,
    model: &str,
) -> Result<String, AiError> {
    let questions = BTreeMap::from([(
        "ok".to_string(),
        Question::Noul {
            instructions: "Is this a connection test?".into(),
        },
    )]);
    let d = provider
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

/// TypeSafe: it only answers typed questions.
pub struct TypeSafe {
    kind: String,
    title: String,
    default_base_url: Option<String>,
    http: Http,
}

impl TypeSafe {
    pub fn new(kind: &str, title: &str, default_base_url: Option<&str>) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            default_base_url: default_base_url.map(str::to_string),
            http: Http::default(),
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for TypeSafe {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn default_base_url(&self) -> Option<&str> {
        self.default_base_url.as_deref()
    }

    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        _req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        Err(AiError::UnsupportedFeature {
            provider: instance.id.into(),
            feature: "text".into(),
            support: CapabilitySupport::Unsupported,
        })
    }

    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        let base = instance.base_url_or(self.default_base_url.as_deref().unwrap_or_default());
        decide(&self.http, instance, &format!("{base}/v1/systemone"), req).await
    }

    async fn test(&self, instance: &ProviderInstance<'_>, model: &str) -> Result<String, AiError> {
        test_by_deciding(self, instance, model).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai_fake::mock;

    fn sample() -> BTreeMap<String, Question> {
        BTreeMap::from([
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
            (
                "size".to_string(),
                Question::Score {
                    instructions: "How big?".into(),
                    levels: vec!["small".into(), "large".into()],
                },
            ),
        ])
    }

    fn at<'a>(base: &'a str, key: Option<&'a str>) -> ProviderInstance<'a> {
        ProviderInstance {
            id: "ts",
            base_url: Some(base),
            key,
        }
    }

    /// TypeSafe only answers typed questions (Jev's `/v1/systemone`); it
    /// can't chat, so its connection check asks one yes/no question.
    #[tokio::test]
    async fn typesafe_only_decides() {
        let (base, seen) = mock(
            "/v1/systemone",
            200,
            json!({"model": "jev-1.13.0", "answers": {
                "risky": {"type": "noul", "noul": 0.8},
                "area": {"type": "choice", "choice": "db", "probabilities": {"ui": 0.1, "db": 0.9}, "confidence": 0.8},
                "size": {"type": "score", "score": 0.7, "probabilities": {"0": 0.3, "1": 0.7}},
                "ok": {"type": "noul", "noul": 0.9}},
                "usage": {"input_tokens": 12, "output_tokens": 0}}),
        )
        .await;
        let typesafe = TypeSafe::new("typesafe", "TypeSafe", Some("https://api.typesafe.ai"));
        let qs = sample();
        let d = typesafe
            .decide(
                &at(&base, Some("tk")),
                &DecideRequest {
                    model: "jev-latest",
                    state: "diff…",
                    questions: &qs,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.8 });
        assert!(matches!(&d.answers["area"], Answer::Choice { choice, .. } if choice == "db"));
        assert!(matches!(&d.answers["size"], Answer::Score { score, .. } if *score == 0.7));
        assert_eq!((d.input_tokens, d.output_tokens), (12, 0));
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["authorization"], "Bearer tk");
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "diff…");
        assert_eq!(
            body["questions"]["risky"],
            json!({"type": "noul", "instructions": "Is this change risky?"})
        );
        assert_eq!(
            body["questions"]["area"]["criteria"],
            json!({"ui": null, "db": null})
        );
        assert_eq!(
            body["questions"]["size"]["criteria"],
            json!(["small", "large"])
        );

        let err = typesafe
            .complete(
                &at(&base, Some("tk")),
                &CompleteRequest {
                    model: "jev",
                    system: None,
                    prompt: "hi",
                    json: false,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AiError::UnsupportedFeature { .. }), "{err}");
        assert_eq!(
            typesafe.test(&at(&base, None), "jev").await.unwrap(),
            "Answered (yes: 90%)"
        );
        assert!(
            !seen.lock().unwrap()[1].1.contains_key("authorization"),
            "no key, no auth header"
        );
    }

    /// A refused key reads as such, and an echoed key is taken out.
    #[tokio::test]
    async fn errors_say_what_went_wrong_without_the_key() {
        let qs = sample();
        let req = DecideRequest {
            model: "jev",
            state: "s",
            questions: &qs,
        };
        let typesafe = TypeSafe::new("typesafe", "TypeSafe", None);
        let (base, _) = mock("/v1/systemone", 401, json!({})).await;
        assert!(matches!(
            typesafe.decide(&at(&base, Some("tk")), &req).await,
            Err(AiError::Auth { .. })
        ));
        let (base, _) = mock("/v1/systemone", 500, json!({"error": "bad key tk-123"})).await;
        let err = typesafe
            .decide(&at(&base, Some("tk-123")), &req)
            .await
            .unwrap_err();
        assert!(!err.to_string().contains("tk-123"), "{err}");
    }
}
