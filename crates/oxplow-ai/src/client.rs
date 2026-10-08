//! Calling models through providers: the [`ModelProvider`] interface every
//! provider implements (`oxplow-ai-providers` has the built-ins: Anthropic
//! Messages, OpenAI-compatible chat, OpenRouter, TypeSafe), the request
//! and answer types, and the [`ModelProviders`] registry, keyed by the
//! declared `ai_provider` id that `ai.yaml`'s `kind:` names. Hand-rolled
//! on reqwest: a few small request shapes, and no Rust crate speaks Jev.
//! See `.context/ai-providers.md`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

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

/// A configured provider as a call sees it: its `ai.yaml` id (what errors
/// name), its `baseUrl` override, and its key.
#[derive(Debug, Clone, Copy)]
pub struct ProviderInstance<'a> {
    pub id: &'a str,
    pub base_url: Option<&'a str>,
    pub key: Option<&'a str>,
}

impl ProviderInstance<'_> {
    /// Its API base: the configured `baseUrl`, else `default`.
    pub fn base_url_or(&self, default: &str) -> String {
        self.base_url
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or(default)
            .trim_end_matches('/')
            .to_string()
    }
}

/// Generate text: `prompt` (with an optional `system` prompt) to `model`.
/// With `json`, ask for a JSON object reply where the API supports it.
#[derive(Debug, Clone, Copy)]
pub struct CompleteRequest<'a> {
    pub model: &'a str,
    pub system: Option<&'a str>,
    pub prompt: &'a str,
    pub json: bool,
}

/// Answer typed `questions` about `state` with `model`.
#[derive(Debug, Clone, Copy)]
pub struct DecideRequest<'a> {
    pub model: &'a str,
    pub state: &'a str,
    pub questions: &'a BTreeMap<String, Question>,
}

/// One kind of model provider oxplow can talk to: a built-in an extension
/// declares as an `ai_provider` implementation.
#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    /// The registry key: the declared id `ai.yaml`'s `kind:` names.
    fn kind(&self) -> &str;
    /// How a person names it (its declaration's title).
    fn title(&self) -> &str;
    /// Its API base when an instance names none; `None` when every instance
    /// must (`baseUrl`).
    fn default_base_url(&self) -> Option<&str>;
    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError>;
    /// By default, a JSON-answer prompt on [`Self::complete`].
    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        decide_via_chat(self, instance, req).await
    }
    /// One small call to check the key, URL and model name: the reply.
    async fn test(&self, instance: &ProviderInstance<'_>, model: &str) -> Result<String, AiError> {
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
        Ok(c.text.trim().chars().take(200).collect())
    }
}

/// Answer typed questions with a chat model: a prompt asking for a JSON
/// reply, then [`parse_answers`].
pub async fn decide_via_chat<P: ModelProvider + ?Sized>(
    provider: &P,
    instance: &ProviderInstance<'_>,
    req: &DecideRequest<'_>,
) -> Result<Decision, AiError> {
    let (state, questions) = (req.state, req.questions);
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
    let c = provider
        .complete(
            instance,
            &CompleteRequest {
                model: req.model,
                system: Some(system),
                prompt: &prompt,
                json: true,
            },
        )
        .await?;
    let reply: serde_json::Value =
        serde_json::from_str(strip_fences(&c.text)).map_err(|e| AiError::BadResponse {
            provider: instance.id.to_string(),
            detail: format!("the model's answer wasn't JSON ({e})"),
        })?;
    let answers = parse_answers(instance.id, &reply["answers"], questions, "probability")?;
    Ok(Decision {
        answers,
        input_tokens: c.input_tokens,
        output_tokens: c.output_tokens,
    })
}

/// The HTTP client the providers share: one JSON POST, its status read
/// into an [`AiError`] naming the provider.
#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Http {
    pub async fn post(
        &self,
        provider: &str,
        url: &str,
        headers: Vec<(&'static str, String)>,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AiError> {
        let id = || provider.to_string();
        let mut req = self.client.post(url).json(&body);
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
}

/// An `Authorization: Bearer` header for `key`, or none.
pub fn bearer(key: Option<&str>) -> Vec<(&'static str, String)> {
    key.map(|k| vec![("authorization", format!("Bearer {k}"))])
        .unwrap_or_default()
}

/// A provider kind nothing registers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no model provider kind `{kind}` (registered: {})", registered.join(", "))]
pub struct UnknownProviderKind {
    pub kind: String,
    pub registered: Vec<String>,
}

/// The model providers the project's extensions declare, by kind. Cloning
/// shares them.
#[derive(Clone, Default)]
pub struct ModelProviders {
    providers: Arc<RwLock<BTreeMap<String, Arc<dyn ModelProvider>>>>,
}

impl ModelProviders {
    /// Replace what's registered.
    pub fn set(&self, providers: Vec<Arc<dyn ModelProvider>>) {
        *self.providers.write().unwrap_or_else(|e| e.into_inner()) = providers
            .into_iter()
            .map(|p| (p.kind().to_string(), p))
            .collect();
    }

    /// Every registered kind, sorted.
    pub fn kinds(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn get(&self, kind: &str) -> Result<Arc<dyn ModelProvider>, UnknownProviderKind> {
        let providers = self.providers.read().unwrap_or_else(|e| e.into_inner());
        providers
            .get(kind)
            .cloned()
            .ok_or_else(|| UnknownProviderKind {
                kind: kind.to_string(),
                registered: providers.keys().cloned().collect(),
            })
    }
}

/// Read one answer per question from `answers`. Parsed by hand rather than
/// through `Answer`'s derive: a dependency enables serde_json's
/// `arbitrary_precision`, which breaks numbers inside internally tagged
/// enums. Jev names a noul's probability `noul`; the chat prompt asks for
/// `probability`.
pub fn parse_answers(
    provider: &str,
    answers: &serde_json::Value,
    questions: &BTreeMap<String, Question>,
    noul_key: &str,
) -> Result<BTreeMap<String, Answer>, AiError> {
    let bad = |detail: String| AiError::BadResponse {
        provider: provider.to_string(),
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

/// Models sometimes wrap JSON in a ```json fence.
pub fn strip_fences(text: &str) -> &str {
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

    struct Echo;

    #[async_trait::async_trait]
    impl ModelProvider for Echo {
        fn kind(&self) -> &str {
            "echo"
        }
        fn title(&self) -> &str {
            "Echo"
        }
        fn default_base_url(&self) -> Option<&str> {
            None
        }
        async fn complete(
            &self,
            _: &ProviderInstance<'_>,
            req: &CompleteRequest<'_>,
        ) -> Result<Completion, AiError> {
            // Answers every question it was asked, from the prompt.
            assert!(req.json && req.system.is_some());
            Ok(Completion {
                text: "```json\n{\"answers\": {\"risky\": {\"type\": \"noul\", \"probability\": 0.3}}}\n```".into(),
                input_tokens: 50,
                output_tokens: 20,
            })
        }
    }

    fn instance() -> ProviderInstance<'static> {
        ProviderInstance {
            id: "p",
            base_url: None,
            key: None,
        }
    }

    /// A provider that only completes answers typed questions through a
    /// JSON prompt, fenced or not.
    #[tokio::test]
    async fn decide_defaults_to_a_json_prompt() {
        let questions = BTreeMap::from([(
            "risky".to_string(),
            Question::Noul {
                instructions: "Is this change risky?".into(),
            },
        )]);
        let d = Echo
            .decide(
                &instance(),
                &DecideRequest {
                    model: "m",
                    state: "diff…",
                    questions: &questions,
                },
            )
            .await
            .unwrap();
        assert_eq!(d.answers["risky"], Answer::Noul { probability: 0.3 });
        assert_eq!((d.input_tokens, d.output_tokens), (50, 20));
    }

    #[test]
    fn an_unknown_kind_names_the_registered() {
        let r = ModelProviders::default();
        r.set(vec![Arc::new(Echo)]);
        assert_eq!(r.kinds(), ["echo"]);
        assert_eq!(
            r.get("nope").err().unwrap(),
            UnknownProviderKind {
                kind: "nope".into(),
                registered: vec!["echo".into()]
            }
        );
    }

    #[test]
    fn an_instances_base_url_overrides_the_default() {
        assert_eq!(instance().base_url_or("https://x/v1/"), "https://x/v1");
        let custom = ProviderInstance {
            base_url: Some(" http://h:1/v1/ "),
            ..instance()
        };
        assert_eq!(custom.base_url_or("https://x"), "http://h:1/v1");
    }
}
