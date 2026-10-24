//! The Rust provider kinds an `ai_provider` declaration names
//! (`provider: <kind>`): each a thin mapping onto one of Rig's provider
//! selections, and `typesafe`, which Rig doesn't speak.

use oxplow_ai::client::{AiError, ProviderInstance};
use rig_core::catalog::{Catalog, ModelFacts};
use rig_core::providers::openai::wire::{Auth, Dialect, Route};
use rig_core::providers::openai::OpenAIConfig;
use rig_core::providers::registry::{Format, ProviderConfig, ProviderId};
use rig_core::wire::Secret;

/// Any OpenAI-compatible server (LM Studio, vLLM, LiteLLM, a proxy): Chat
/// Completions at the instance's own URL, which has no default.
const COMPATIBLE: Dialect = Dialect::gateway("openai_compatible", "", "OPENAI_COMPATIBLE_API_KEY");

/// What Rig speaks for a kind.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Speaks {
    /// One of Rig's registered selections: its vendor and format.
    Rig {
        vendor: &'static str,
        format: Format,
    },
    /// [`COMPATIBLE`], at the instance's URL.
    Compatible,
    /// TypeSafe's Jev: typed decisions only.
    Jev,
}

/// One kind.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub name: &'static str,
    pub(crate) speaks: Speaks,
    /// Whether its models list is the provider's own (a hosted service),
    /// so Rig's built-in catalog of it is the models it serves. A local
    /// server's are whatever it has pulled.
    pub(crate) hosted: bool,
    /// Whether it answers typed questions natively for some model.
    pub decides_natively: bool,
}

/// Every kind, by name.
pub const KINDS: &[Kind] = &[
    rig("anthropic", "anthropic", Format::Anthropic, true),
    rig("deepseek", "deepseek", Format::OpenAi, true),
    rig("gemini", "gcp.gemini", Format::Gemini, true),
    rig("groq", "groq", Format::OpenAi, true),
    rig("llamacpp", "llamacpp", Format::OpenAi, false),
    rig("mistral", "mistral", Format::OpenAi, true),
    rig("ollama", "ollama", Format::OpenAi, false),
    rig("openai", "openai", Format::OpenAi, true),
    Kind {
        name: "openai_compatible",
        speaks: Speaks::Compatible,
        hosted: false,
        decides_natively: false,
    },
    Kind {
        // Jev models answer typed questions natively at `/systemone`.
        decides_natively: true,
        ..rig("openrouter", "openrouter", Format::OpenAi, true)
    },
    Kind {
        name: "typesafe",
        speaks: Speaks::Jev,
        hosted: true,
        decides_natively: true,
    },
    rig("xai", "xai", Format::OpenAi, true),
];

const fn rig(name: &'static str, vendor: &'static str, format: Format, hosted: bool) -> Kind {
    Kind {
        name,
        speaks: Speaks::Rig { vendor, format },
        hosted,
        decides_natively: false,
    }
}

/// A kind no declaration may name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no AI provider kind `{kind}` (oxplow's kinds: {})", names().join(", "))]
pub struct UnknownKind {
    pub kind: String,
}

/// Every kind's name.
pub fn names() -> Vec<&'static str> {
    KINDS.iter().map(|k| k.name).collect()
}

/// The kind `name`.
pub fn kind(name: &str) -> Result<&'static Kind, UnknownKind> {
    KINDS
        .iter()
        .find(|k| k.name == name)
        .ok_or_else(|| UnknownKind { kind: name.into() })
}

impl Kind {
    /// Rig's selection, for a kind Rig speaks as one.
    pub(crate) fn provider_id(&self) -> Option<ProviderId> {
        match self.speaks {
            Speaks::Rig { vendor, format } => ProviderId::new(vendor, format),
            Speaks::Compatible | Speaks::Jev => None,
        }
    }

    /// The API base Rig uses for it when an instance names none; `None`
    /// for one every instance must name, and for one Rig doesn't speak.
    pub fn rig_base_url(&self) -> Option<String> {
        let id = self.provider_id()?;
        Some(id.config("")?.base_url().to_string()).filter(|u| !u.is_empty())
    }

    /// Rig's configuration for `instance`, at `base_url`, with the
    /// instance's key (never the environment's). With no key the auth
    /// header is left out (a local server).
    pub(crate) fn config(
        &self,
        instance: &ProviderInstance<'_>,
        base_url: &str,
    ) -> Result<ProviderConfig, AiError> {
        let key = Secret::from(instance.key.unwrap_or_default());
        let config = match self.speaks {
            Speaks::Rig { .. } => self
                .provider_id()
                .and_then(|id| id.config(key))
                .ok_or_else(|| AiError::BadRequest {
                    provider: instance.id.into(),
                    detail: format!("Rig has no `{}` provider", self.name),
                })?,
            Speaks::Compatible => ProviderConfig::OpenAi(OpenAIConfig::with_key(&COMPATIBLE, key)),
            Speaks::Jev => {
                return Err(AiError::UnsupportedFeature {
                    provider: instance.id.into(),
                    feature: "text".into(),
                    support: oxplow_ai::capabilities::CapabilitySupport::Unsupported,
                })
            }
        };
        let config = config.with_base_url(base_url);
        Ok(match config {
            ProviderConfig::OpenAi(openai) if instance.key.is_none() => {
                ProviderConfig::OpenAi(openai.with_auth(Auth::OptionalBearer))
            }
            other => other,
        })
    }

    /// Whether its completions go to Chat Completions, where a JSON reply
    /// is asked for with `response_format`.
    pub(crate) fn chat_route(config: &ProviderConfig) -> bool {
        matches!(config, ProviderConfig::OpenAi(openai) if openai.completion_route() == Route::Chat)
    }

    /// The facts Rig's catalog has for `model` of this kind, which its wire
    /// encodes with (a default output limit); none for a model it doesn't
    /// list.
    pub(crate) fn facts(&self, model: &str) -> ModelFacts {
        match self.provider_id() {
            Some(id) => Catalog::builtin().facts(id, model),
            None => ModelFacts::default(),
        }
    }
}
