//! Cores for the `ai` command module: Settings → AI. Providers and role
//! assignments live in the user-global `ai.yaml`; keys go to the keychain
//! and are never returned, only whether each is set. UI only: agents can
//! read roles (`list_ai_roles` on MCP) but not change settings or keys.
//! See `.context/ai-providers.md`.

use oxplow_app::ai_service::{AiServiceError, AiSettings, ProviderConfig, Role, RoleBinding};
use oxplow_app::Services;

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
    Ok(svc.ai.settings()?)
}

/// Add or replace a provider; a non-empty `key` is stored in the keychain.
pub async fn save_ai_provider(
    svc: &Services,
    provider: ProviderConfig,
    key: Option<String>,
) -> Result<AiSettings, IpcError> {
    svc.ai.save_provider(provider, key)?;
    Ok(svc.ai.settings()?)
}

pub async fn remove_ai_provider(svc: &Services, id: String) -> Result<AiSettings, IpcError> {
    svc.ai.remove_provider(&id)?;
    Ok(svc.ai.settings()?)
}

/// Assign `role` to a provider + model, or unassign it with no binding.
/// A role's binding is config (an `ai.roles.<role>` row in Settings'
/// effective-config view), so the change is announced as `ConfigChanged`
/// like every other config write.
pub async fn set_ai_role(
    svc: &Services,
    role: Role,
    binding: Option<RoleBinding>,
) -> Result<AiSettings, IpcError> {
    svc.ai.set_role(role, binding)?;
    svc.events.emit(oxplow_app::OxplowEvent::ConfigChanged);
    Ok(svc.ai.settings()?)
}

/// One small call to check a provider's key, URL and model. Returns the reply.
pub async fn test_ai_provider(
    svc: &Services,
    id: String,
    model: String,
) -> Result<String, IpcError> {
    Ok(svc.ai.test_provider(&id, &model).await?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn providers_roles_and_keys_round_trip_without_exposing_keys() {
        let (svc, _dir) = crate::test_support::services();
        let st = crate::dispatch(
            "save_ai_provider",
            json!({ "provider": { "id": "or", "kind": "openrouter" }, "key": "sk-secret" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(st["providers"][0]["keySet"], true);
        assert!(!st.to_string().contains("sk-secret"));

        let st = crate::dispatch(
            "set_ai_role",
            json!({ "role": "summarize", "binding": { "provider": "or", "model": "m" } }),
            &svc,
        )
        .await
        .unwrap();
        let summarize = st["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["role"] == "summarize")
            .unwrap()
            .clone();
        assert_eq!(summarize["binding"]["model"], "m");

        let err = crate::dispatch("remove_ai_provider", json!({ "id": "or" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");

        crate::dispatch(
            "set_ai_role",
            json!({ "role": "summarize", "binding": null }),
            &svc,
        )
        .await
        .unwrap();
        let st = crate::dispatch("remove_ai_provider", json!({ "id": "or" }), &svc)
            .await
            .unwrap();
        assert_eq!(st["providers"], json!([]));
        let st = crate::dispatch("ai_settings", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(st["roles"].as_array().unwrap().len(), 6);
    }

    /// A role's binding is config (an `ai.roles.<role>` row in Settings'
    /// effective-config view), so changing it announces `ConfigChanged`
    /// like every other config write — the view refreshes the one way.
    #[tokio::test]
    async fn setting_a_role_announces_a_config_change() {
        let (svc, _dir) = crate::test_support::services();
        crate::dispatch(
            "save_ai_provider",
            json!({ "provider": { "id": "or", "kind": "openrouter" }, "key": "sk-secret" }),
            &svc,
        )
        .await
        .unwrap();
        let mut rx = svc.events.subscribe();
        crate::dispatch(
            "set_ai_role",
            json!({ "role": "summarize", "binding": { "provider": "or", "model": "m" } }),
            &svc,
        )
        .await
        .unwrap();
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            saw |= matches!(event, oxplow_app::OxplowEvent::ConfigChanged);
        }
        assert!(saw, "set_ai_role emitted no ConfigChanged");
    }

    #[tokio::test]
    async fn testing_an_unknown_provider_is_invalid() {
        let (svc, _dir) = crate::test_support::services();
        let err = crate::dispatch("test_ai_provider", json!({ "id": "x", "model": "m" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
