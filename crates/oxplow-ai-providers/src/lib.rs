//! The built-in model providers (`ai_provider` implementations,
//! `.context/ai-providers.md`): Anthropic Messages, any OpenAI-compatible
//! chat API (OpenAI, Ollama, LM Studio, vLLM, LiteLLM), OpenRouter and
//! TypeSafe (Jev). Core registers the ones a project's extensions declare,
//! each under its declared id — the `kind:` an `ai.yaml` provider names.

mod anthropic;
mod openai_compatible;
mod openrouter;
mod scripted;
mod typesafe;

pub use scripted::{scripted, FUNCTIONS};

use std::sync::Arc;

use oxplow_ai::client::ModelProvider;

/// The provider a built-in `entry` is, registered as `id` (named `title`)
/// and configured with `config`; `None` for an entry that isn't one, an
/// error for a config that doesn't hold.
pub fn built_in(
    entry: &str,
    id: &str,
    title: &str,
    config: &serde_json::Value,
) -> Option<Result<Arc<dyn ModelProvider>, String>> {
    let (kind, title) = (id.to_string(), title.to_string());
    Some(Ok(match entry {
        "oxplow:anthropic" => Arc::new(anthropic::Anthropic::new(kind, title)),
        "oxplow:openai-compatible" => {
            return Some(
                openai_compatible::OpenaiCompatible::new(kind, title, config).map(|p| {
                    let p: Arc<dyn ModelProvider> = Arc::new(p);
                    p
                }),
            )
        }
        "oxplow:openrouter" => Arc::new(openrouter::Openrouter::new(kind, title)),
        "oxplow:typesafe" => Arc::new(typesafe::Typesafe::new(kind, title)),
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_built_in_is_its_declared_kind() {
        for (entry, id, base) in [
            (
                "oxplow:anthropic",
                "anthropic",
                Some("https://api.anthropic.com"),
            ),
            (
                "oxplow:openrouter",
                "openrouter",
                Some("https://openrouter.ai/api/v1"),
            ),
            (
                "oxplow:typesafe",
                "typesafe",
                Some("https://api.typesafe.ai"),
            ),
        ] {
            let p = built_in(entry, id, id, &serde_json::json!({}))
                .unwrap()
                .unwrap();
            assert_eq!((p.kind(), p.default_base_url()), (id, base));
        }
        let openai = built_in(
            "oxplow:openai-compatible",
            "openai",
            "OpenAI",
            &serde_json::json!({ "baseUrl": "https://api.openai.com/v1" }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(openai.default_base_url(), Some("https://api.openai.com/v1"));
        let local = built_in(
            "oxplow:openai-compatible",
            "openai_compatible",
            "Local",
            &serde_json::json!({}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            local.default_base_url(),
            None,
            "every instance names its URL"
        );
        assert!(built_in(
            "oxplow:openai-compatible",
            "x",
            "x",
            &serde_json::json!({ "nope": 1 })
        )
        .unwrap()
        .is_err());
        assert!(built_in("oxplow:claude-code", "claude", "c", &serde_json::json!({})).is_none());
    }
}
