//! Calling models through providers: the [`ModelProvider`] interface every
//! provider implements (`oxplow-ai-providers` has the built-ins: Anthropic
//! Messages, OpenAI-compatible chat, OpenRouter, TypeSafe), the request
//! and answer types, and the [`ModelProviders`] registry, keyed by the
//! declared `ai_provider` id that `ai.yaml`'s `kind:` names. Hand-rolled
//! on reqwest: a few small request shapes, and no Rust crate speaks Jev.
//! See `.context/ai-providers.md`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub use oxplow_domain::run::Cancel;

use crate::capabilities::{
    CapabilitySnapshotV1, CapabilitySupport, ModelCatalogV1, SchemaVersionError,
};
use crate::parts::{
    FinishReasonV1, MessageRoleV1, PartV1, StreamDeltaV1, TypedRequestV1, TypedResponseV1,
    UsageSourceV1, UsageV1,
};
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
    #[error("{provider}: model feature `{feature}` has {support:?} support")]
    UnsupportedFeature {
        provider: String,
        feature: String,
        support: CapabilitySupport,
    },
    #[error("{0}")]
    UnsupportedSchemaVersion(#[from] SchemaVersionError),
    /// Its program — a provider written as a script — runs only once a
    /// person approves it, and nothing was sent.
    #[error("{provider}: {message} (Settings → Data → Programs)")]
    Unapproved { provider: String, message: String },
    /// The call was cancelled: its connection was dropped, and `partial`
    /// is what had arrived.
    #[error("{provider}: cancelled")]
    Cancelled {
        provider: String,
        partial: Box<TypedResponseV1>,
    },
    /// A stream sent nothing for [`STREAM_IDLE`].
    #[error("{provider}: the stream sent nothing for {idle_secs}s")]
    Timeout { provider: String, idle_secs: u64 },
}

/// How long a stream may stay silent between events before the call is
/// given up.
pub const STREAM_IDLE: Duration = Duration::from_secs(60);

/// How long a stream's reader waits for more once an event arrived, so
/// what arrives together is handled together.
pub const STREAM_COALESCE: Duration = Duration::from_millis(50);

/// How often a call looks at its [`Cancel`] while it waits on the wire.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Where a streamed reply's pieces go as they arrive.
pub type OnDelta<'a> = &'a mut (dyn FnMut(StreamDeltaV1) + Send);

/// Why a call stopped waiting before its reply was whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    Idle,
}

impl Stopped {
    /// The error it is, for `provider`, with what had arrived.
    pub fn error(self, provider: &str, partial: TypedResponseV1) -> AiError {
        match self {
            Stopped::Cancelled => AiError::Cancelled {
                provider: provider.to_string(),
                partial: Box::new(partial),
            },
            Stopped::Idle => AiError::Timeout {
                provider: provider.to_string(),
                idle_secs: STREAM_IDLE.as_secs(),
            },
        }
    }
}

/// `future`, unless `cancel` is asked first (looked at every
/// [`CANCEL_POLL`]); dropping it is what stops the work, an HTTP call's
/// connection included.
pub async fn unless_cancelled<T>(
    cancel: &Cancel,
    future: impl std::future::Future<Output = T>,
) -> Result<T, Stopped> {
    tokio::pin!(future);
    loop {
        if cancel.is_cancelled() {
            return Err(Stopped::Cancelled);
        }
        tokio::select! {
            out = &mut future => return Ok(out),
            _ = tokio::time::sleep(CANCEL_POLL) => {}
        }
    }
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
    /// Enumerate locally available metadata. This must not make paid probes
    /// or use credentials; a provider that only supports text completion and
    /// decide has no known catalog.
    async fn discover_models(
        &self,
        _instance: &ProviderInstance<'_>,
    ) -> Result<ModelCatalogV1, AiError> {
        Ok(ModelCatalogV1::unknown())
    }
    /// Discover metadata without making a model call. A provider that only
    /// supports text completion and decide returns an unknown snapshot and
    /// never probes a potentially billable endpoint.
    async fn discover(
        &self,
        _instance: &ProviderInstance<'_>,
        model: &str,
    ) -> Result<CapabilitySnapshotV1, AiError> {
        Ok(CapabilitySnapshotV1::unknown(model))
    }
    /// Complete a typed, text-only request through the provider's text completion.
    /// Providers with native image or tool transport can override this method.
    async fn complete_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
    ) -> Result<TypedResponseV1, AiError> {
        complete_typed_via_text(self, instance, req).await
    }
    /// [`Self::complete_typed`], its reply handed to `on_delta` as it
    /// arrives. Until `cancel` is asked: then the call is dropped (its
    /// connection with it) and the error is [`AiError::Cancelled`] with
    /// what had arrived. By default the whole reply arrives at once, at
    /// the end; a provider that speaks a streaming transport overrides it.
    async fn stream_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        stream_whole(self, instance, req, cancel, on_delta).await
    }
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

/// The default stream: [`ModelProvider::complete_typed`], dropped if
/// `cancel` is asked, its whole reply handed over at the end.
pub async fn stream_whole<P: ModelProvider + ?Sized>(
    provider: &P,
    instance: &ProviderInstance<'_>,
    req: &TypedRequestV1,
    cancel: &Cancel,
    on_delta: OnDelta<'_>,
) -> Result<TypedResponseV1, AiError> {
    let response = unless_cancelled(cancel, provider.complete_typed(instance, req))
        .await
        .map_err(|stop| stop.error(instance.id, TypedResponseV1::partial(Vec::new())))??;
    for delta in StreamDeltaV1::of(&response.parts) {
        on_delta(delta);
    }
    Ok(response)
}

/// Adapt ordered text parts to the provider's text completion without discarding unsupported parts.
pub async fn complete_typed_via_text<P: ModelProvider + ?Sized>(
    provider: &P,
    instance: &ProviderInstance<'_>,
    req: &TypedRequestV1,
) -> Result<TypedResponseV1, AiError> {
    req.validate_version()?;
    let unsupported = |feature: &str, support| AiError::UnsupportedFeature {
        provider: instance.id.to_string(),
        feature: feature.to_string(),
        support,
    };
    if !req.tools.is_empty() {
        return Err(unsupported("toolCalling", CapabilitySupport::Unknown));
    }
    let mut system = Vec::new();
    let mut prompt = Vec::new();
    let mut saw_user = false;
    for message in &req.messages {
        let text = message
            .parts
            .iter()
            .map(|part| match part {
                PartV1::Text { text } => Ok(text.as_str()),
                PartV1::Image { .. } => Err(("image", CapabilitySupport::Unknown)),
                PartV1::ToolCall { .. } | PartV1::ToolResult { .. } => {
                    Err(("toolCalling", CapabilitySupport::Unknown))
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|(feature, support)| unsupported(feature, support))?
            .join("");
        match message.role {
            MessageRoleV1::System if !saw_user => system.push(text),
            MessageRoleV1::User if !saw_user => {
                prompt.push(text);
                saw_user = true;
            }
            MessageRoleV1::System => {
                return Err(unsupported("messageOrder", CapabilitySupport::Unsupported));
            }
            MessageRoleV1::User | MessageRoleV1::Assistant | MessageRoleV1::Tool => {
                return Err(unsupported("conversation", CapabilitySupport::Unsupported));
            }
        }
    }
    if !saw_user {
        return Err(unsupported("userMessage", CapabilitySupport::Unsupported));
    }
    let system = system.join("\n\n");
    let prompt = prompt.join("");
    let result = provider
        .complete(
            instance,
            &CompleteRequest {
                model: &req.model,
                system: if system.is_empty() {
                    None
                } else {
                    Some(&system)
                },
                prompt: &prompt,
                json: false,
            },
        )
        .await?;
    let input_tokens = u64::try_from(result.input_tokens)
        .ok()
        .filter(|count| *count > 0);
    let output_tokens = u64::try_from(result.output_tokens)
        .ok()
        .filter(|count| *count > 0);
    Ok(TypedResponseV1 {
        schema_version: 1,
        parts: vec![PartV1::Text { text: result.text }],
        finish_reason: FinishReasonV1::Unknown,
        usage: UsageV1 {
            input_tokens,
            output_tokens,
            source: UsageSourceV1::Unknown,
        },
    })
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
    streaming: reqwest::Client,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build HTTP client with redirects disabled"),
            streaming: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build streaming HTTP client with redirects disabled"),
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
        let headers = headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        self.post_with(provider, url, headers, body).await
    }

    /// [`Self::post`] with headers named at run time (a scripted
    /// provider's).
    pub async fn post_with(
        &self,
        provider: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AiError> {
        let id = || provider.to_string();
        let mut req = self.client.post(url).json(&body);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| AiError::Transport {
            provider: id(),
            message: e.without_url().to_string(),
        })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| AiError::Transport {
            provider: id(),
            message: e.without_url().to_string(),
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

impl Http {
    /// A POST whose reply is a server-sent event stream: the status is read
    /// first (the same errors as [`Self::post_with`], the body read for
    /// the message), then [`SseStream`] reads the events as they come. No
    /// total timeout: a stream lasts as long as the model writes; its
    /// reader gives up after [`STREAM_IDLE`] of silence.
    pub async fn post_stream(
        &self,
        provider: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: serde_json::Value,
    ) -> Result<SseStream, AiError> {
        let id = || provider.to_string();
        let mut req = self
            .streaming
            .post(url)
            .header("accept", "text/event-stream")
            .json(&body);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| AiError::Transport {
            provider: id(),
            message: e.without_url().to_string(),
        })?;
        let status = resp.status().as_u16();
        if !(200..=299).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(match status {
                401 | 403 => AiError::Auth { provider: id() },
                429 => AiError::RateLimited { provider: id() },
                _ => AiError::Http {
                    provider: id(),
                    status,
                    body: text.chars().take(300).collect(),
                },
            });
        }
        Ok(SseStream {
            provider: id(),
            resp,
            buf: Vec::new(),
            done: false,
        })
    }
}

/// One server-sent event: its `event:` name, if any, and its `data:`
/// lines joined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// A reply read as server-sent events.
pub struct SseStream {
    provider: String,
    resp: reqwest::Response,
    buf: Vec<u8>,
    done: bool,
}

impl SseStream {
    /// The events that have arrived, at least one (waiting for the wire
    /// as long as it takes); `None` once the stream ended.
    pub async fn next_batch(&mut self) -> Result<Option<Vec<SseEvent>>, AiError> {
        loop {
            let events = take_events(&mut self.buf, self.done);
            if !events.is_empty() {
                return Ok(Some(events));
            }
            if self.done {
                return Ok(None);
            }
            match self.resp.chunk().await.map_err(|e| AiError::Transport {
                provider: self.provider.clone(),
                message: e.without_url().to_string(),
            })? {
                Some(bytes) => self.buf.extend(bytes.iter().filter(|b| **b != b'\r')),
                None => self.done = true,
            }
        }
    }

    /// [`Self::next_batch`], unless `cancel` is asked or the wire stays
    /// silent for [`STREAM_IDLE`]. Once one event arrived, whatever else
    /// arrives within [`STREAM_COALESCE`] joins it, so a model writing a
    /// token at a time is read in a few batches a second, not one per token.
    pub async fn next_batch_until(
        &mut self,
        cancel: &Cancel,
    ) -> Result<Result<Option<Vec<SseEvent>>, AiError>, Stopped> {
        let first =
            match unless_cancelled(cancel, tokio::time::timeout(STREAM_IDLE, self.next_batch()))
                .await?
            {
                Ok(Ok(Some(batch))) => batch,
                Ok(other) => return Ok(other),
                Err(_) => return Err(Stopped::Idle),
            };
        let mut batch = first;
        let until = tokio::time::Instant::now() + STREAM_COALESCE;
        // Reading a chunk is cancel-safe: one cut off by the deadline was
        // never taken off the wire.
        while let Ok(more) = tokio::time::timeout_at(until, self.next_batch()).await {
            match more {
                Ok(Some(more)) => batch.extend(more),
                Ok(None) => break,
                Err(e) => return Ok(Err(e)),
            }
        }
        Ok(Ok(Some(batch)))
    }
}

/// The whole events at the front of `buf`, taken off it (a blank line ends
/// one). With `ended`, what is left is the last. `buf` holds no `\r`: the
/// reader drops them, so `\r\n` reads as `\n`.
fn take_events(buf: &mut Vec<u8>, ended: bool) -> Vec<SseEvent> {
    let cut = buf.windows(2).rposition(|w| w == b"\n\n");
    let whole = match cut {
        // Cut at a blank line, which never splits a UTF-8 character: what
        // stays may end mid-character and waits for the rest of it.
        Some(at) => {
            let rest = buf.split_off(at + 2);
            let whole = String::from_utf8_lossy(&buf[..at]).into_owned();
            *buf = rest;
            whole
        }
        None if ended && !buf.is_empty() => {
            String::from_utf8_lossy(&std::mem::take(buf)).into_owned()
        }
        None => return Vec::new(),
    };
    whole
        .split("\n\n")
        .filter_map(|block| {
            let mut event = None;
            let mut data: Vec<&str> = Vec::new();
            for line in block.lines() {
                if line.starts_with(':') {
                    continue;
                }
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => event = Some(value.to_string()),
                    "data" => data.push(value),
                    _ => {}
                }
            }
            (!data.is_empty()).then(|| SseEvent {
                event,
                data: data.join("\n"),
            })
        })
        .collect()
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

/// The model providers the project's extensions declare, by kind, and the
/// few the host registers itself (a preview's fixture model). Cloning
/// shares them.
#[derive(Clone, Default)]
pub struct ModelProviders {
    providers: Arc<RwLock<BTreeMap<String, Arc<dyn ModelProvider>>>>,
    host: Arc<RwLock<BTreeMap<String, Arc<dyn ModelProvider>>>>,
}

impl ModelProviders {
    /// Replace what the extensions register; the host's kinds stay.
    pub fn set(&self, providers: Vec<Arc<dyn ModelProvider>>) {
        *self.providers.write().unwrap_or_else(|e| e.into_inner()) = providers
            .into_iter()
            .map(|p| (p.kind().to_string(), p))
            .collect();
    }

    /// Register a kind the host provides, which no activation replaces.
    pub fn register_host(&self, provider: Arc<dyn ModelProvider>) {
        self.host
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(provider.kind().to_string(), provider);
    }

    /// Every registered kind, sorted.
    pub fn kinds(&self) -> Vec<String> {
        let mut kinds: Vec<String> = self
            .providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .chain(self.host.read().unwrap_or_else(|e| e.into_inner()).keys())
            .cloned()
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }

    pub fn get(&self, kind: &str) -> Result<Arc<dyn ModelProvider>, UnknownProviderKind> {
        let found = self
            .providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(kind)
            .cloned()
            .or_else(|| {
                self.host
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(kind)
                    .cloned()
            });
        found.ok_or_else(|| UnknownProviderKind {
            kind: kind.to_string(),
            registered: self.kinds(),
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

    /// A kind the host registers (a preview's fixture model) isn't the
    /// extensions' to replace: an activation's `set` keeps it.
    #[test]
    fn a_host_kind_outlives_what_extensions_register() {
        let r = ModelProviders::default();
        r.register_host(Arc::new(crate::fixture::FixtureModel::new(vec![
            "hi".into()
        ])));
        r.set(vec![Arc::new(Echo)]);
        assert_eq!(r.kinds(), ["echo", "fixture"]);
        assert_eq!(r.get("fixture").unwrap().kind(), "fixture");
        r.set(Vec::new());
        assert_eq!(r.kinds(), ["fixture"]);
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
    /// A text-only provider that keeps what it was asked and counts calls.
    struct Recorder {
        calls: std::sync::Mutex<Vec<(Option<String>, String, String)>>,
        reply: Completion,
    }

    impl Recorder {
        fn new(input_tokens: i64, output_tokens: i64) -> Self {
            Self {
                calls: Default::default(),
                reply: Completion {
                    text: "answer".into(),
                    input_tokens,
                    output_tokens,
                },
            }
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for Recorder {
        fn kind(&self) -> &str {
            "rec"
        }
        fn title(&self) -> &str {
            "Rec"
        }
        fn default_base_url(&self) -> Option<&str> {
            None
        }
        async fn complete(
            &self,
            _: &ProviderInstance<'_>,
            req: &CompleteRequest<'_>,
        ) -> Result<Completion, AiError> {
            self.calls.lock().unwrap().push((
                req.system.map(str::to_string),
                req.prompt.to_string(),
                req.model.to_string(),
            ));
            Ok(self.reply.clone())
        }
    }

    fn msg(role: MessageRoleV1, parts: Vec<PartV1>) -> crate::parts::MessageV1 {
        crate::parts::MessageV1 { role, parts }
    }

    fn text(t: &str) -> PartV1 {
        PartV1::Text { text: t.into() }
    }

    fn typed(messages: Vec<crate::parts::MessageV1>) -> TypedRequestV1 {
        TypedRequestV1 {
            schema_version: 1,
            model: "m".into(),
            messages,
            tools: Vec::new(),
        }
    }

    /// A provider with no discovery reports unknown, never a guess, and
    /// asks nothing of the model to do it.
    #[tokio::test]
    async fn a_text_only_provider_discovers_unknown_without_a_call() {
        let p = Recorder::new(1, 1);
        let snap = p
            .discover(&instance(), "claude-looks-capable")
            .await
            .unwrap();
        assert_eq!(snap, CapabilitySnapshotV1::unknown("claude-looks-capable"));
        assert_eq!(snap.image, CapabilitySupport::Unknown);
        assert_eq!(
            p.discover_models(&instance()).await.unwrap(),
            ModelCatalogV1::unknown()
        );
        assert!(p.calls.lock().unwrap().is_empty());
    }

    /// System text and one user message become the text call; their text
    /// is kept, and text-only replies carry no finish reason or usage source.
    #[tokio::test]
    async fn typed_text_runs_over_the_text_path() {
        let p = Recorder::new(12, 4);
        let req = typed(vec![
            msg(MessageRoleV1::System, vec![text("be "), text("brief")]),
            msg(MessageRoleV1::System, vec![text("really")]),
            msg(MessageRoleV1::User, vec![text("hel"), text("lo")]),
        ]);
        let r = p.complete_typed(&instance(), &req).await.unwrap();
        assert_eq!(
            p.calls.lock().unwrap()[0],
            (
                Some("be brief\n\nreally".into()),
                "hello".into(),
                "m".into()
            )
        );
        assert_eq!(r.parts, vec![text("answer")]);
        assert_eq!(r.finish_reason, FinishReasonV1::Unknown);
        assert_eq!(r.usage.source, UsageSourceV1::Unknown);
        assert_eq!(
            (r.usage.input_tokens, r.usage.output_tokens),
            (Some(12), Some(4))
        );
    }

    /// The text path's zero sentinel means "not reported", not a known zero.
    #[tokio::test]
    async fn text_path_sentinel_counts_stay_unknown() {
        let p = Recorder::new(0, -1);
        let req = typed(vec![msg(MessageRoleV1::User, vec![text("q")])]);
        let r = p.complete_typed(&instance(), &req).await.unwrap();
        assert_eq!((r.usage.input_tokens, r.usage.output_tokens), (None, None));
    }

    /// What the text path cannot carry is refused, naming the feature, and
    /// nothing is sent.
    #[tokio::test]
    async fn the_text_adapter_refuses_what_it_would_drop() {
        let p = Recorder::new(1, 1);
        let user = |parts| msg(MessageRoleV1::User, parts);
        let image = PartV1::Image {
            asset_ref: "a".into(),
            media_type: "image/png".into(),
        };
        let call = PartV1::ToolCall {
            id: "c".into(),
            name: "n".into(),
            arguments: serde_json::json!({}),
        };
        let mut with_tools = typed(vec![user(vec![text("q")])]);
        with_tools.tools.push(crate::parts::ToolDefinitionV1 {
            name: "n".into(),
            description: None,
            parameters: serde_json::json!({}),
        });
        let cases = [
            (
                typed(vec![user(vec![text("q"), image])]),
                "image",
                CapabilitySupport::Unknown,
            ),
            (
                typed(vec![user(vec![call])]),
                "toolCalling",
                CapabilitySupport::Unknown,
            ),
            (with_tools, "toolCalling", CapabilitySupport::Unknown),
            (
                typed(vec![user(vec![text("a")]), user(vec![text("b")])]),
                "conversation",
                CapabilitySupport::Unsupported,
            ),
            (
                typed(vec![
                    user(vec![text("a")]),
                    msg(MessageRoleV1::Assistant, vec![text("b")]),
                ]),
                "conversation",
                CapabilitySupport::Unsupported,
            ),
            (
                typed(vec![msg(MessageRoleV1::System, vec![text("only system")])]),
                "userMessage",
                CapabilitySupport::Unsupported,
            ),
        ];
        for (req, want_feature, want_support) in cases {
            match p.complete_typed(&instance(), &req).await {
                Err(AiError::UnsupportedFeature {
                    feature, support, ..
                }) => {
                    assert_eq!((feature.as_str(), support), (want_feature, want_support));
                }
                other => panic!("expected an unsupported {want_feature}: {other:?}"),
            }
        }
        let stale = TypedRequestV1 {
            schema_version: 2,
            ..typed(vec![msg(MessageRoleV1::User, vec![text("q")])])
        };
        assert!(matches!(
            p.complete_typed(&instance(), &stale).await,
            Err(AiError::UnsupportedSchemaVersion(_))
        ));
        assert!(p.calls.lock().unwrap().is_empty(), "nothing was sent");
    }

    fn sse(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_string),
            data: data.to_string(),
        }
    }

    /// Events are whole only at a blank line, however the bytes arrive:
    /// comments are skipped, `data:` lines join, `\r\n` reads as `\n`,
    /// and a character split across chunks is whole once the rest comes.
    #[test]
    fn events_are_cut_at_blank_lines_however_the_bytes_arrive() {
        let mut buf = Vec::new();
        buf.extend(b"event: a\ndata: one\n\n: keep-alive\n\ndata: tw".iter());
        assert_eq!(take_events(&mut buf, false), [sse(Some("a"), "one")]);
        assert_eq!(buf, b"data: tw");
        // The rest of the event, with a character split across chunks.
        let snow = "\u{2603}".as_bytes();
        buf.extend(b"o\ndata: ".iter().chain(&snow[..1]));
        assert!(take_events(&mut buf, false).is_empty());
        buf.extend(snow[1..].iter().chain(b"\n\n"));
        assert_eq!(take_events(&mut buf, false), [sse(None, "two\n\u{2603}")]);
        assert!(buf.is_empty());
        // The last event needs no blank line once the stream ended.
        buf.extend(b"data:[DONE]");
        assert!(take_events(&mut buf, false).is_empty());
        assert_eq!(take_events(&mut buf, true), [sse(None, "[DONE]")]);
    }

    /// The reader hands over events as they arrive, then `None` at the end;
    /// `\r\n` from the wire reads as `\n`.
    #[tokio::test]
    async fn a_stream_reads_events_as_they_arrive() {
        let (base, seen, _) = oxplow_ai_fake::mock_sse(
            "/s",
            vec!["data: a\r\n\r\n".into(), "data: b\n\ndata: c\n\n".into()],
            Duration::from_millis(20),
            Duration::ZERO,
        )
        .await;
        let mut stream = Http::default()
            .post_stream(
                "p",
                &format!("{base}/s"),
                vec![],
                serde_json::json!({ "x": 1 }),
            )
            .await
            .unwrap();
        let mut got = Vec::new();
        while let Some(batch) = stream.next_batch().await.unwrap() {
            got.extend(batch.into_iter().map(|e| e.data));
        }
        assert_eq!(got, ["a", "b", "c"]);
        let (_, headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["accept"], "text/event-stream");
        assert_eq!(body, serde_json::json!({ "x": 1 }));
    }

    /// A status that isn't 2xx is the same error a plain call gives.
    #[tokio::test]
    async fn a_stream_reads_the_status_first() {
        let (base, _) = oxplow_ai_fake::mock("/s", 429, serde_json::json!({})).await;
        let err = Http::default()
            .post_stream("p", &format!("{base}/s"), vec![], serde_json::json!({}))
            .await
            .err()
            .unwrap();
        assert_eq!(
            err,
            AiError::RateLimited {
                provider: "p".into()
            }
        );
    }

    /// Cancelling a stream mid-way stops the wait at once and drops the
    /// connection: the server stops being read from.
    #[tokio::test]
    async fn a_cancelled_stream_stops_and_lets_go_of_the_connection() {
        let chunks = (0..50).map(|i| format!("data: {i}\n\n")).collect();
        let (base, _, written) = oxplow_ai_fake::mock_sse(
            "/s",
            chunks,
            Duration::from_millis(50),
            Duration::from_secs(30),
        )
        .await;
        let mut stream = Http::default()
            .post_stream("p", &format!("{base}/s"), vec![], serde_json::json!({}))
            .await
            .unwrap();
        let cancel = Cancel::default();
        let first = stream.next_batch_until(&cancel).await.unwrap().unwrap();
        assert_eq!(first.unwrap()[0].data, "0");
        cancel.cancel();
        let started = std::time::Instant::now();
        assert_eq!(
            stream.next_batch_until(&cancel).await.err(),
            Some(Stopped::Cancelled)
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        drop(stream);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let at_drop = written.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            written.load(std::sync::atomic::Ordering::SeqCst),
            at_drop,
            "the server stops writing once the client let go"
        );
        assert!(at_drop < 50);
    }

    /// By default a provider "streams" its whole reply at once, at the end;
    /// cancelled first, it reports nothing arrived.
    #[tokio::test]
    async fn the_default_stream_is_the_whole_reply_at_once() {
        let p = Recorder::new(4, 2);
        let req = typed(vec![msg(MessageRoleV1::User, vec![text("hi")])]);
        let mut deltas = Vec::new();
        let response = p
            .stream_typed(&instance(), &req, &Cancel::default(), &mut |d| {
                deltas.push(d)
            })
            .await
            .unwrap();
        assert_eq!(
            deltas,
            [StreamDeltaV1::Text {
                text: "answer".into()
            }]
        );
        assert_eq!(response.parts, [text("answer")]);

        let cancel = Cancel::default();
        cancel.cancel();
        let err = p
            .stream_typed(&instance(), &req, &cancel, &mut |_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AiError::Cancelled { partial, .. } if partial.parts.is_empty()),
            "{err:?}"
        );
    }
}
