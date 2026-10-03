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

/// A person adds another instance of an extension's `provider`
/// (`instance` = `<extension>/<instance id>`), the project's or — `scope:
/// global` — their own on this machine: off until they configure and
/// enable it.
pub async fn add_provider_instance(
    svc: &Services,
    instance: String,
    provider: String,
    scope: oxplow_app::providers::Scope,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    svc.providers
        .add_instance(&Actor::Human, &instance, &provider, scope)
        .await?;
    Ok(svc.providers.list().await)
}

/// A person removes `instance`: it stops, and its config entry and its
/// credentials on this machine go.
pub async fn remove_provider_instance(
    svc: &Services,
    instance: String,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    svc.providers
        .remove_instance(&Actor::Human, &instance)
        .await?;
    Ok(svc.providers.list().await)
}

/// Set (or, with no value, forget) one of `instance`'s credentials in
/// this machine's keychain; the instance restarts on it. UI only: an
/// agent never sets a credential.
pub async fn set_instance_credential(
    svc: &Services,
    instance: String,
    name: String,
    value: Option<String>,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    svc.providers
        .set_credential(&instance, &name, value.as_deref())?;
    svc.providers.credential_changed(&instance).await;
    Ok(svc.providers.list().await)
}

/// Start signing in for one of `instance`'s credentials (one its
/// provider declares with `oauth:`): the page the person signs in on, to
/// open in their browser. When they have, the token is kept in this
/// machine's keychain, the instance restarts on it, and the renderer
/// hears `credentialChanged`. UI only: an agent never signs in.
pub async fn begin_oauth_sign_in(
    svc: &Services,
    instance: String,
    name: String,
) -> Result<String, IpcError> {
    Ok(svc.providers.begin_sign_in(&instance, &name).await?)
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
        // P9.B1: another instance, its credential, its removal — each
        // refused the same way while no extension declares the provider.
        for (name, input) in [
            (
                "add_provider_instance",
                json!({ "instance": "tracker/fake_two", "provider": "fake", "scope": "project" }),
            ),
            (
                "set_instance_credential",
                json!({ "instance": "tracker/fake", "name": "TOKEN", "value": "x" }),
            ),
            (
                "remove_provider_instance",
                json!({ "instance": "tracker/fake" }),
            ),
            (
                "begin_oauth_sign_in",
                json!({ "instance": "tracker/fake", "name": "TOKEN" }),
            ),
        ] {
            let refused = crate::dispatch(name, input, &svc).await.unwrap_err();
            assert_eq!(refused.code, "INVALID", "{name}: {}", refused.message);
            assert!(
                refused.message.contains("tracker/fake"),
                "{name}: {}",
                refused.message
            );
        }
        assert!(svc.config.read().unwrap().extension_instances.is_empty());
    }
}
