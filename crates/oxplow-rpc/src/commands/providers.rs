//! Cores for Settings → Integrations (P5.D4, `.context/providers.md`):
//! extension providers' instances on this machine. UI only — enabling an
//! instance runs a program, so it is a person's (the config key is
//! human-only and `oxplow.plugin.enable` is human-only too).

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

/// A person turns their global instance `instance` off in this project
/// only: the project gets its own entry, off, replacing it here.
pub async fn turn_off_provider_instance_here(
    svc: &Services,
    instance: String,
) -> Result<Vec<ProviderInstanceView>, IpcError> {
    svc.providers.off_here(&Actor::Human, &instance).await?;
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
/// provider declares with `oauth:`), its redirect coming back to
/// `redirect_port` on the person's machine, where the desktop shell
/// listens: the page the person signs in on, to open in their browser,
/// and the sign-in's number (its news names it; it cancels it). UI only:
/// an agent never signs in.
pub async fn begin_oauth_sign_in(
    svc: &Services,
    instance: String,
    name: String,
    redirect_port: u16,
) -> Result<oxplow_app::providers::BegunSignIn, IpcError> {
    Ok(svc
        .providers
        .begin_sign_in(&instance, &name, redirect_port)
        .await?)
}

/// End sign-in `sign_in` (its row was left, or the browser never opened):
/// nothing of it is kept, and its news says it was cancelled. UI only.
pub async fn cancel_oauth_sign_in(
    svc: &Services,
    instance: String,
    name: String,
    sign_in: u32,
) -> Result<(), IpcError> {
    svc.providers
        .cancel_sign_in(&instance, &name, sign_in)
        .await;
    Ok(())
}

/// The redirect the shell caught for that sign-in (`redirect`: the path
/// and query the browser asked for). Not that sign-in's: refused, and it
/// waits on. Its: the token is kept in this machine's keychain, the
/// instance restarts on it, and the renderer hears `credentialChanged`.
/// UI only: an agent never signs in.
pub async fn complete_oauth_sign_in(
    svc: &Services,
    instance: String,
    name: String,
    redirect: String,
) -> Result<oxplow_app::providers::SignInCompletion, IpcError> {
    Ok(svc
        .providers
        .complete_sign_in(&instance, &name, &redirect)
        .await?)
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
                "turn_off_provider_instance_here",
                json!({ "instance": "tracker/fake" }),
            ),
            (
                "begin_oauth_sign_in",
                json!({ "instance": "tracker/fake", "name": "TOKEN", "redirectPort": 8124 }),
            ),
            (
                "complete_oauth_sign_in",
                json!({ "instance": "tracker/fake", "name": "TOKEN", "redirect": "/callback" }),
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
