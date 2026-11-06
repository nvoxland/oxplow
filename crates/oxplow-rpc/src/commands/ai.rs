//! Cores for the `ai` command module: Settings → AI's reads. Providers and
//! role assignments live in the user-global `ai.yaml`; keys go to the
//! keychain and are never returned, only whether each is set. Changing them
//! is the `oxplow.ai.*` commands (`oxplow-app`'s `commands/ai_settings.rs`),
//! a person's alone. See `.context/ai-providers.md`.

use oxplow_app::ai_service::{AiServiceError, AiSettings};
use oxplow_app::Services;
use oxplow_domain::AiCallId;

use crate::error::IpcError;

impl From<AiServiceError> for IpcError {
    fn from(e: AiServiceError) -> Self {
        match e {
            AiServiceError::Secret(_) => IpcError::internal(e.to_string()),
            _ => IpcError::invalid(e.to_string()),
        }
    }
}

pub async fn ai_settings(svc: &Services) -> Result<AiSettings, IpcError> {
    Ok(svc.ai.settings().await?)
}

/// What model call `id` was asked or answered, capped; `None` when it has
/// no such body or retention removed it. The person's read: any call.
pub async fn read_ai_call(
    svc: &Services,
    id: AiCallId,
    body: oxplow_app::ai_calls::AiCallBody,
) -> Result<Option<oxplow_app::event_bodies::EventBody>, IpcError> {
    use oxplow_app::ai_calls::{read, AiCallBodyError};
    read(svc, id.value(), body, None)
        .await
        .map_err(|e| match e {
            AiCallBodyError::Storage(e) => e.into(),
            other => IpcError::invalid(other.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn the_settings_list_the_roles_without_any_provider() {
        let (svc, _dir) = crate::test_support::services().await;
        let st = crate::dispatch("ai_settings", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(st["providers"], json!([]));
        // The four built-ins.
        assert_eq!(st["roles"].as_array().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn reading_an_unknown_model_call_is_invalid() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch("read_ai_call", json!({ "id": 9, "body": "request" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
