//! [`RigProvider`]: oxplow's [`ModelProvider`] over Rig, for every kind
//! Rig speaks.

use std::time::Duration;

use futures::StreamExt;
use oxplow_ai::capabilities::{
    CapabilitySnapshotV1, CapabilitySupport, KnownLimit, ModelCatalogV1,
};
use oxplow_ai::client::{
    decide_via_chat, unless_cancelled, AiError, Cancel, CompleteRequest, Completion, DecideRequest,
    Decision, Http, ModelProvider, OnDelta, ProviderInstance, Stopped, STREAM_IDLE,
};
use oxplow_ai::parts::{DeltaFold, PartV1, StreamDeltaV1, TypedRequestV1, TypedResponseV1};
use rig_core::catalog::{Catalog, ModelSpec};
use rig_core::completion::{AssistantContent, CompletionRequest, Message};
use rig_core::http_client::DynHttpClient;
use rig_core::operation::Completion as CompletionOp;
use rig_core::providers::registry::ProviderConfig;
use rig_core::streaming::{Item, StreamEvent};
use rig_core::DynModel;
use serde_json::json;

use crate::convert;
use crate::errors::ai_error;
use crate::kinds::Kind;

/// How long a whole (unstreamed) reply may take.
const UNARY_TIMEOUT: Duration = Duration::from_secs(120);

/// The output limit a Messages request sends for a model Rig's catalog
/// doesn't list.
const MESSAGES_MAX_TOKENS: u64 = 4096;

/// What a JSON reply asks for, beside the API's own switch where it has one.
const JSON_ONLY: &str = "Reply with a single JSON object and nothing else.";

/// An HTTP client for Rig that never follows a redirect (a request can't
/// carry its key or content to another host); `timeout` bounds a whole
/// call, else only connecting is bounded (a stream lasts as long as the
/// model writes; its reader gives up after [`STREAM_IDLE`] of silence).
fn http_client(timeout: Option<Duration>) -> DynHttpClient {
    let builder = rig_reqwest::reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(rig_reqwest::reqwest::redirect::Policy::none());
    let builder = match timeout {
        Some(timeout) => builder.timeout(timeout),
        None => builder,
    };
    let client = builder
        .build()
        .expect("build HTTP client with redirects disabled");
    DynHttpClient::new(rig_reqwest::ReqwestClient::from(client))
}

/// A declared `ai_provider` of a kind Rig speaks.
pub struct RigProvider {
    /// The declared id `ai.yaml`'s `kind:` names.
    id: String,
    title: String,
    default_base_url: Option<String>,
    kind: &'static Kind,
    unary: DynHttpClient,
    streaming: DynHttpClient,
    /// For OpenRouter's Jev models.
    jev: Http,
}

impl RigProvider {
    pub(crate) fn new(
        kind: &'static Kind,
        id: &str,
        title: &str,
        default_base_url: Option<&str>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            default_base_url: default_base_url.map(str::to_string),
            kind,
            unary: http_client(Some(UNARY_TIMEOUT)),
            streaming: http_client(None),
            jev: Http::default(),
        }
    }

    /// The instance's API base: its own, else the declaration's.
    fn base_url(&self, instance: &ProviderInstance<'_>) -> Result<String, AiError> {
        let base = instance.base_url_or(self.default_base_url.as_deref().unwrap_or_default());
        if base.is_empty() {
            return Err(AiError::BadRequest {
                provider: instance.id.into(),
                detail: format!("{} needs a base URL (Settings → AI)", self.title),
            });
        }
        Ok(base)
    }

    /// Rig's model handle for `model` on `instance`, sending through
    /// `http`, and whether it speaks Chat Completions.
    fn model(
        &self,
        instance: &ProviderInstance<'_>,
        model: &str,
        http: DynHttpClient,
    ) -> Result<(DynModel<CompletionOp>, bool), AiError> {
        let base = self.base_url(instance)?;
        let config = self.kind.config(instance, &base)?;
        let chat = Kind::chat_route(&config);
        let facts = self.kind.facts(model);
        let handle = match config {
            // The Messages API needs `max_tokens`; Rig knows it for the
            // models its catalog lists, and this is the rest's.
            ProviderConfig::Anthropic(anthropic) => {
                let mut handle = anthropic.connect(http).completion(model);
                handle.wire = handle.wire.with_facts(facts);
                if handle.wire.default_max_tokens.is_none() {
                    handle.wire = handle.wire.with_default_max_tokens(MESSAGES_MAX_TOKENS);
                }
                handle.erase()
            }
            other => other.completion_model(model, http, facts),
        };
        Ok((handle, chat))
    }

    /// The handle a caller drives itself (the Assistant): `model` on
    /// `instance`, streaming-capable, with the instance's key.
    pub fn completion_model(
        &self,
        instance: &ProviderInstance<'_>,
        model: &str,
    ) -> Result<DynModel<CompletionOp>, AiError> {
        self.model(instance, model, self.streaming.clone())
            .map(|(model, _)| model)
    }

    /// Rig's catalog entry for `model`, for a kind it has one for.
    fn spec(&self, model: &str) -> Option<&'static ModelSpec> {
        let id = self.kind.provider_id()?;
        Catalog::builtin().get(id, model).map(|found| found.spec)
    }
}

fn support(known: bool) -> CapabilitySupport {
    if known {
        CapabilitySupport::Supported
    } else {
        CapabilitySupport::Unsupported
    }
}

fn tokens(limit: Option<u32>) -> Option<KnownLimit> {
    limit.map(|value| KnownLimit {
        value: u64::from(value),
        unit: "tokens".into(),
    })
}

/// What Rig's catalog says of a model. Streaming isn't a model fact it
/// keeps, so it stays unknown.
fn snapshot(model: &str, spec: &ModelSpec) -> CapabilitySnapshotV1 {
    CapabilitySnapshotV1 {
        schema_version: 1,
        model: Some(model.to_string()),
        model_known: true,
        text: support(spec.input.text),
        image: support(spec.input.image),
        tool_calling: support(spec.tools),
        streaming: CapabilitySupport::Unknown,
        context_tokens: tokens(spec.context_window),
        output_tokens: tokens(spec.max_output_tokens),
    }
}

/// The text of a reply's parts.
fn text_of(parts: &[PartV1]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            PartV1::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn count(tokens: Option<u64>) -> i64 {
    tokens.and_then(|t| i64::try_from(t).ok()).unwrap_or(0)
}

#[async_trait::async_trait]
impl ModelProvider for RigProvider {
    fn kind(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn default_base_url(&self) -> Option<&str> {
        self.default_base_url.as_deref()
    }

    /// A hosted service's models, from Rig's built-in catalog (models.dev
    /// data compiled in): no call, no key. A local server's are whatever
    /// it has pulled, so unknown.
    async fn discover_models(
        &self,
        _instance: &ProviderInstance<'_>,
    ) -> Result<ModelCatalogV1, AiError> {
        let Some(id) = self.kind.provider_id().filter(|_| self.kind.hosted) else {
            return Ok(ModelCatalogV1::unknown());
        };
        let mut seen = std::collections::BTreeSet::new();
        let models = Catalog::builtin()
            .iter()
            .filter(|spec| spec.provider.vendor() == id.vendor() && !spec.deprecated)
            .filter(|spec| seen.insert(spec.id.clone()))
            .map(|spec| snapshot(&spec.id, spec))
            .collect();
        Ok(ModelCatalogV1 {
            schema_version: 1,
            catalog_known: true,
            models,
        })
    }

    async fn discover(
        &self,
        _instance: &ProviderInstance<'_>,
        model: &str,
    ) -> Result<CapabilitySnapshotV1, AiError> {
        Ok(self
            .spec(model)
            .map(|spec| snapshot(model, spec))
            .unwrap_or_else(|| CapabilitySnapshotV1::unknown(model)))
    }

    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        let (model, chat) = self.model(instance, req.model, self.unary.clone())?;
        let mut system = req.system.unwrap_or_default().trim().to_string();
        if req.json {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(JSON_ONLY);
        }
        let mut request = CompletionRequest::new(Message::user(req.prompt));
        if !system.is_empty() {
            request = request.preamble(system);
        }
        if req.json && chat {
            request =
                request.additional_params(json!({ "response_format": { "type": "json_object" } }));
        }
        let reply = model
            .call(request)
            .await
            .map_err(|e| ai_error(instance.id, instance.key, e))?;
        let typed = convert::response(instance.id, &reply)?;
        Ok(Completion {
            text: text_of(&typed.parts),
            input_tokens: count(typed.usage.input_tokens),
            output_tokens: count(typed.usage.output_tokens),
        })
    }

    async fn complete_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
    ) -> Result<TypedResponseV1, AiError> {
        let request = convert::request(instance.id, req)?;
        let (model, _) = self.model(instance, &req.model, self.unary.clone())?;
        let reply = model
            .call(request)
            .await
            .map_err(|e| ai_error(instance.id, instance.key, e))?;
        convert::response(instance.id, &reply)
    }

    /// Rig's stream, as it arrives: each text fragment a `text` delta, each
    /// tool call one `toolCall` delta once its arguments are whole. Cancel
    /// drops the stream, its connection with it.
    async fn stream_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        let id = instance.id;
        let request = convert::request(id, req)?;
        let (model, _) = self.model(instance, &req.model, self.streaming.clone())?;
        let mut stream = model
            .stream(request)
            .map_err(|e| ai_error(id, instance.key, e))?;
        let mut fold = DeltaFold::default();
        loop {
            let next =
                unless_cancelled(cancel, tokio::time::timeout(STREAM_IDLE, stream.next())).await;
            let item = match next {
                Err(stop) => return Err(stop.error(id, fold.into_partial())),
                Ok(Err(_silent)) => return Err(Stopped::Idle.error(id, fold.into_partial())),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(Err(e)))) => return Err(ai_error(id, instance.key, e)),
                Ok(Ok(Some(Ok(item)))) => item,
            };
            let delta = match item {
                Item::Event(StreamEvent::Text { text, .. }) if !text.is_empty() => {
                    Some(StreamDeltaV1::Text { text })
                }
                Item::Event(StreamEvent::End {
                    content: content @ AssistantContent::ToolCall(_),
                    ..
                }) => match convert::part(id, &content)? {
                    Some(PartV1::ToolCall {
                        id,
                        name,
                        arguments,
                    }) => Some(StreamDeltaV1::ToolCall {
                        id,
                        name,
                        arguments,
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(delta) = delta {
                fold.push(&delta);
                on_delta(delta);
            }
        }
        let reply = stream
            .finish()
            .await
            .map_err(|e| ai_error(id, instance.key, e))?;
        convert::response(id, &reply)
    }

    /// OpenRouter's Jev models answer natively at `/systemone`; any other
    /// model through a JSON prompt.
    async fn decide(
        &self,
        instance: &ProviderInstance<'_>,
        req: &DecideRequest<'_>,
    ) -> Result<Decision, AiError> {
        if self.kind.decides_natively && req.model.contains("jev") {
            let base = self.base_url(instance)?;
            return crate::jev::decide(&self.jev, instance, &format!("{base}/systemone"), req)
                .await;
        }
        decide_via_chat(self, instance, req).await
    }
}
