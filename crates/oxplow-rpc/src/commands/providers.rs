//! Cores for Settings → Integrations (P5.D4, `.context/providers.md`):
//! extension providers' instances on this machine. UI only — enabling an
//! instance runs a program, so it is a person's (the config key is
//! human-only and `plugin.enable` is human-only too).

use oxplow_app::providers::ProviderInstanceView;
use oxplow_app::Services;
use oxplow_domain::{Actor, Json};

use crate::error::IpcError;

/// Every declared provider and configured instance, with its health.
pub async fn list_provider_instances(
    svc: &Services,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    Ok(svc.providers.list().await)
}

/// Check `instance` against `config` — consent, spawn, handshake,
/// `check` — enabling and writing nothing. The outcome is the view's
/// state.
pub async fn check_provider_instance(
    svc: &Services,
    instance: String,
    config: Json,
) -> Result<ProviderInstanceView, IpcError> {
    Ok(svc.providers.check_instance(&instance, config.0).await?)
}

/// Save `instance`'s config and enable or disable it, as the person.
/// Enabling checks first: an unapproved or unconfigured instance is
/// refused and nothing is written.
pub async fn set_provider_instance(
    svc: &Services,
    instance: String,
    enabled: bool,
    config: Json,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    svc.providers
        .set_instance(&Actor::Human, &instance, enabled, config.0)
        .await?;
    Ok(svc.providers.list().await)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn provider_instances_dispatch() {
        let (svc, _dir) = crate::test_support::services();
        let listed = crate::dispatch("list_provider_instances", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(listed, json!([]));
        // No extension declares it: refused, nothing written.
        let refused = crate::dispatch(
            "set_provider_instance",
            json!({ "instance": "tracker/fake", "enabled": true, "config": { "team": "core" } }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, "INVALID", "{}", refused.message);
        assert!(refused.message.contains("tracker/fake"));
        assert!(svc.config.read().unwrap().extension_instances.is_empty());
    }
}
