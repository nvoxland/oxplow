//! Calling models: chat completions (Anthropic Messages, or any
//! OpenAI-compatible endpoint: OpenAI, OpenRouter, Ollama, LM Studio,
//! vLLM) and typed decisions (Jev's `/v1/systemone`, natively or via
//! OpenRouter; otherwise a JSON-answer prompt on a chat model).
//! Hand-rolled on reqwest: three small request shapes, and no Rust crate
//! speaks Jev. See `.context/ai-providers.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{ProviderConfig, ProviderKind};

#[derive(Clone, Debug, thiserror::Error, PartialEq)]
pub enum AiError {
    #[error("{provider}: authentication failed; check the key in Settings → AI")]
    Auth { provider: String },
    #[error("{provider}: rate limited, try again shortly")]
    RateLimited { provider: String },
    #[error("{provider}: {status}: {body}")]
    Http {
        provider: String,
        status: u16,
        body: String,
    },
    #[error("{provider}: {message}")]
    Transport { provider: String, message: String },
    #[error("{provider}: unexpected response: {detail}")]
    BadResponse { provider: String, detail: String },
}

/// A chat model's reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// One typed question for `decide`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Question {
    /// Yes/no; the answer is the probability of yes.
    Noul { instructions: String },
    /// Pick one of `options`.
    Choice {
        instructions: String,
        options: Vec<String>,
    },
    /// Place on an ordered scale (`levels`, lowest first).
    Score {
        instructions: String,
        levels: Vec<String>,
    },
}

/// An answer to one question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Answer {
    Noul {
        probability: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
    },
    /// `score` is the probability-weighted level index (0 = first level).
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub answers: BTreeMap<String, Answer>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// Talks to one provider.
pub struct Client {
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Client {
    /// Send `prompt` (with an optional `system` prompt) to `model`. With
    /// `json`, ask for a JSON object reply where the API supports it.
    pub async fn complete(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        model: &str,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<Completion, AiError> {
        match provider.kind {
            ProviderKind::Anthropic => {
                self.anthropic(provider, key, model, system, prompt, json)
                    .await
            }
            ProviderKind::Typesafe => Err(AiError::BadResponse {
                provider: provider.id.clone(),
                detail: "TypeSafe (Jev) only answers typed questions; use it for the decide role"
                    .into(),
            }),
            _ => {
                self.openai_chat(provider, key, model, system, prompt, json)
                    .await
            }
        }
    }

    /// Answer typed `questions` about `state`.
    pub async fn decide(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        model: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Decision, AiError> {
        let native = match provider.kind {
            ProviderKind::Typesafe => Some(format!("{}/v1/systemone", base_url(provider))),
            ProviderKind::Openrouter if model.contains("jev") => {
                Some(format!("{}/systemone", base_url(provider)))
            }
            _ => None,
        };
        match native {
            Some(url) => {
                self.systemone(provider, &url, key, model, state, questions)
                    .await
            }
            None => {
                self.decide_via_chat(provider, key, model, state, questions)
                    .await
            }
        }
    }

    async fn post(
        &self,
        provider: &ProviderConfig,
        url: &str,
        headers: Vec<(&'static str, String)>,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AiError> {
        let id = || provider.id.clone();
        let mut req = self.http.post(url).json(&body);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| AiError::Transport {
            provider: id(),
            message: e.to_string(),
        })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| AiError::Transport {
            provider: id(),
            message: e.to_string(),
        })?;
        match status {
            200..=299 => serde_json::from_str(&text).map_err(|e| AiError::BadResponse {
                provider: id(),
                detail: e.to_string(),
            }),
            401 | 403 => Err(AiError::Auth { provider: id() }),
            429 => Err(AiError::RateLimited { provider: id() }),
            _ => Err(AiError::Http {
                provider: id(),
                status,
                body: text.chars().take(300).collect(),
            }),
        }
    }

    async fn anthropic(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        model: &str,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<Completion, AiError> {
        let mut system = system.unwrap_or_default().to_string();
        if json {
            system.push_str("\n\nReply with a single JSON object and nothing else.");
        }
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}],
        });
        if !system.trim().is_empty() {
            body["system"] = serde_json::Value::String(system.trim().to_string());
        }
        let mut headers = vec![("anthropic-version", "2023-06-01".to_string())];
        if let Some(k) = key {
            headers.push(("x-api-key", k.to_string()));
        }
        let url = format!("{}/v1/messages", base_url(provider));
        let v = self.post(provider, &url, headers, body).await?;
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
                provider: provider.id.clone(),
                detail: "no content".into(),
            })?;
        Ok(Completion {
            text,
            input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
            output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
        })
    }

    async fn openai_chat(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        model: &str,
        system: Option<&str>,
        prompt: &str,
        json: bool,
    ) -> Result<Completion, AiError> {
        let mut messages = Vec::new();
        if let Some(sys) = system.filter(|s| !s.trim().is_empty()) {
            messages.push(serde_json::json!({"role": "system", "content": sys}));
        }
        messages.push(serde_json::json!({"role": "user", "content": prompt}));
        let mut body = serde_json::json!({"model": model, "messages": messages});
        if json {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        let url = format!("{}/chat/completions", base_url(provider));
        let v = self.post(provider, &url, bearer(key), body).await?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| AiError::BadResponse {
                provider: provider.id.clone(),
                detail: "no choices[0].message.content".into(),
            })?
            .to_string();
        Ok(Completion {
            text,
            input_tokens: v["usage"]["prompt_tokens"].as_i64().unwrap_or(0),
            output_tokens: v["usage"]["completion_tokens"].as_i64().unwrap_or(0),
        })
    }

    async fn systemone(
        &self,
        provider: &ProviderConfig,
        url: &str,
        key: Option<&str>,
        model: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Decision, AiError> {
        let qs: serde_json::Map<String, serde_json::Value> = questions
            .iter()
            .map(|(name, q)| {
                let v = match q {
                    Question::Noul { instructions } => serde_json::json!({"type": "noul", "instructions": instructions}),
                    Question::Choice { instructions, options } => serde_json::json!({
                        "type": "choice",
                        "instructions": instructions,
                        "criteria": options.iter().map(|o| (o.clone(), serde_json::Value::Null)).collect::<serde_json::Map<_, _>>(),
                    }),
                    Question::Score { instructions, levels } => serde_json::json!({
                        "type": "score", "instructions": instructions, "criteria": levels,
                    }),
                };
                (name.clone(), v)
            })
            .collect();
        let body = serde_json::json!({"model": model, "state": state, "questions": qs});
        let v = self.post(provider, url, bearer(key), body).await?;
        let answers = parse_answers(provider, &v["answers"], questions, "noul")?;
        Ok(Decision {
            answers,
            input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
            output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
        })
    }

    async fn decide_via_chat(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        model: &str,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Decision, AiError> {
        let system = "You answer typed questions about the given state. Reply with JSON only, shaped \
            {\"answers\": {\"<question name>\": <answer>}}. Answers by type: \
            noul → {\"type\":\"noul\",\"probability\":<0..1 that the answer is yes>}; \
            choice → {\"type\":\"choice\",\"choice\":\"<one option>\",\"probabilities\":{\"<option>\":<0..1>}}; \
            score → {\"type\":\"score\",\"score\":<level index, 0 = first level, fractional ok>,\"probabilities\":{\"<index>\":<0..1>}}.";
        let mut prompt = format!("State:\n{state}\n\nQuestions:");
        for (name, q) in questions {
            match q {
                Question::Noul { instructions } => {
                    prompt.push_str(&format!("\n- {name} (noul): {instructions}"))
                }
                Question::Choice {
                    instructions,
                    options,
                } => prompt.push_str(&format!(
                    "\n- {name} (choice from {}): {instructions}",
                    options.join(", ")
                )),
                Question::Score {
                    instructions,
                    levels,
                } => prompt.push_str(&format!(
                    "\n- {name} (score; levels in order: {}): {instructions}",
                    levels
                        .iter()
                        .enumerate()
                        .map(|(i, l)| format!("{i}={l}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        let c = self
            .complete(provider, key, model, Some(system), &prompt, true)
            .await?;
        let reply: serde_json::Value =
            serde_json::from_str(strip_fences(&c.text)).map_err(|e| AiError::BadResponse {
                provider: provider.id.clone(),
                detail: format!("the model's answer wasn't JSON ({e})"),
            })?;
        let answers = parse_answers(provider, &reply["answers"], questions, "probability")?;
        Ok(Decision {
            answers,
            input_tokens: c.input_tokens,
            output_tokens: c.output_tokens,
        })
    }
}

/// Read one answer per question from `answers`. Parsed by hand rather than
/// through `Answer`'s derive: a dependency enables serde_json's
/// `arbitrary_precision`, which breaks numbers inside internally tagged
/// enums. Jev names a noul's probability `noul`; the chat prompt asks for
/// `probability`.
fn parse_answers(
    provider: &ProviderConfig,
    answers: &serde_json::Value,
    questions: &BTreeMap<String, Question>,
    noul_key: &str,
) -> Result<BTreeMap<String, Answer>, AiError> {
    let bad = |detail: String| AiError::BadResponse {
        provider: provider.id.clone(),
        detail,
    };
    let probs = |x: &serde_json::Value| -> BTreeMap<String, f64> {
        x.as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, p)| p.as_f64().map(|p| (k.clone(), p)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = BTreeMap::new();
    for name in questions.keys() {
        let a = &answers[name];
        let answer = match a["type"].as_str() {
            Some("noul") => Answer::Noul {
                probability: a[noul_key]
                    .as_f64()
                    .ok_or_else(|| bad(format!("{name}: no {noul_key}")))?,
            },
            Some("choice") => Answer::Choice {
                choice: a["choice"]
                    .as_str()
                    .ok_or_else(|| bad(format!("{name}: no choice")))?
                    .to_string(),
                probabilities: probs(&a["probabilities"]),
            },
            Some("score") => Answer::Score {
                score: a["score"]
                    .as_f64()
                    .ok_or_else(|| bad(format!("{name}: no score")))?,
                probabilities: probs(&a["probabilities"]),
            },
            _ => return Err(bad(format!("no answer for `{name}`"))),
        };
        out.insert(name.clone(), answer);
    }
    Ok(out)
}

/// The provider's API base: its configured `base_url`, else the kind's default.
pub fn base_url(provider: &ProviderConfig) -> String {
    if let Some(b) = provider
        .base_url
        .as_deref()
        .filter(|b| !b.trim().is_empty())
    {
        return b.trim_end_matches('/').to_string();
    }
    match provider.kind {
        ProviderKind::Anthropic => "https://api.anthropic.com",
        ProviderKind::Openai => "https://api.openai.com/v1",
        ProviderKind::Openrouter => "https://openrouter.ai/api/v1",
        ProviderKind::Typesafe => "https://api.typesafe.ai",
        ProviderKind::OpenaiCompatible => "http://localhost:11434/v1",
    }
    .to_string()
}

fn bearer(key: Option<&str>) -> Vec<(&'static str, String)> {
    key.map(|k| vec![("authorization", format!("Bearer {k}"))])
        .unwrap_or_default()
}

/// Models sometimes wrap JSON in a ```json fence.
fn strip_fences(text: &str) -> &str {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    t.strip_suffix("```").unwrap_or(t).trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_ai_fake::mock;
    use serde_json::{json, Value};

    fn provider(kind: ProviderKind, base: &str) -> ProviderConfig {
        ProviderConfig {
            id: "p".into(),
            kind,
            base_url: Some(base.into()),
        }
    }

    #[tokio::test]
    async fn anthropic_messages() {
        let (base, seen) = mock(
            "/v1/messages",
            200,
            json!({"content": [{"type": "text", "text": "hi"}], "usage": {"input_tokens": 7, "output_tokens": 2}}),
        )
        .await;
        let c = Client::default()
            .complete(
                &provider(ProviderKind::Anthropic, &base),
                Some("k1"),
                "claude-x",
                Some("be brief"),
                "hello",
                false,
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

    #[tokio::test]
    async fn openai_compatible_chat_with_optional_key() {
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": "{\"a\":1}"}}], "usage": {"prompt_tokens": 5, "completion_tokens": 3}}),
        )
        .await;
        let c = Client::default()
            .complete(
                &provider(ProviderKind::OpenaiCompatible, &base),
                None,
                "qwen3",
                None,
                "q",
                true,
            )
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

    fn questions() -> BTreeMap<String, Question> {
        let mut q = BTreeMap::new();
        q.insert(
            "risky".to_string(),
            Question::Noul {
                instructions: "Is this change risky?".into(),
            },
        );
        q.insert(
            "area".to_string(),
            Question::Choice {
                instructions: "Which area?".into(),
                options: vec!["ui".into(), "db".into()],
            },
        );
        q
    }

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
        let d = Client::default()
            .decide(
                &provider(ProviderKind::Typesafe, &base),
                Some("tk"),
                "jev-latest",
                "diff…",
                &questions(),
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

    #[tokio::test]
    async fn decide_falls_back_to_a_json_prompt_on_chat_models() {
        let reply = json!({"answers": {"risky": {"type": "noul", "probability": 0.3}, "area": {"type": "choice", "choice": "ui", "probabilities": {"ui": 0.7, "db": 0.3}}}});
        let (base, seen) = mock(
            "/chat/completions",
            200,
            json!({"choices": [{"message": {"content": reply.to_string()}}], "usage": {"prompt_tokens": 50, "completion_tokens": 20}}),
        )
        .await;
        let d = Client::default()
            .decide(
                &provider(ProviderKind::OpenaiCompatible, &base),
                None,
                "qwen3",
                "diff…",
                &questions(),
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
        let err = Client::default()
            .complete(
                &provider(ProviderKind::Openai, &base),
                Some("x"),
                "m",
                None,
                "q",
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            AiError::Auth {
                provider: "p".into()
            }
        );
        let (base, _) = mock("/chat/completions", 429, json!({})).await;
        let err = Client::default()
            .complete(
                &provider(ProviderKind::Openai, &base),
                Some("x"),
                "m",
                None,
                "q",
                false,
            )
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
