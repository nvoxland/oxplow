//! Settings → AI as commands: `oxplow.ai.set_provider` adds or replaces a
//! provider (and stores its key in the keychain), `remove_provider` takes
//! one away, `set_role` binds a role to a provider and model (or unbinds
//! it) and `test_provider` makes one small call to check a provider.
//! Anyone runs them: a person, their agent, Settings → AI as a form page.
//! `set_provider`'s `key` is a secret (`writeOnly`): never in a record,
//! only that one was given is.
//! `External`: the settings are the user-global `ai.yaml` and the OS
//! keychain, outside oxplow's database. Reading them is
//! `v_ai_provider` / `v_ai_role`.

use std::sync::Arc;

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_domain::ai::AiRestriction;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::ops::Op;
use super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::ai_service::AiServiceError;
use crate::events::OxplowEvent;

pub const SET_PROVIDER: &str = "oxplow.ai.set_provider";
pub const REMOVE_PROVIDER: &str = "oxplow.ai.remove_provider";
pub const SET_ROLE: &str = "oxplow.ai.set_role";
pub const TEST_PROVIDER: &str = "oxplow.ai.test_provider";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetProviderInput {
    /// Its id, which is its kind (`anthropic`); also its keychain entry.
    /// Left out, the kind.
    #[serde(default)]
    pub id: Option<String>,
    /// What it is: a declared AI provider kind (`anthropic`, `openai`,
    /// `openai_compatible`, `openrouter`).
    pub kind: String,
    /// Its API base: required for a kind with no default, an override for the others.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Its API key, stored in the OS keychain and never read back. Left
    /// out (or empty), the stored one stays. A secret (`writeOnly`).
    #[serde(default)]
    #[schemars(extend("writeOnly" = true))]
    pub key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderInput {
    /// The provider.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetRoleInput {
    /// A built-in role (`main`, `summarize`, `decide`) or an extension's AI
    /// profile (`acme/review`).
    pub role: String,
    /// The provider it uses; with `model` left out, the role is unbound.
    #[serde(default)]
    pub provider: Option<String>,
    /// The model on that provider.
    #[serde(default)]
    pub model: Option<String>,
    /// What this binding is forbidden: `tools`, `sql`, `streaming`, `images`.
    #[serde(default)]
    pub restrict: Vec<AiRestriction>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestProviderInput {
    /// The provider to call.
    pub id: String,
    /// The model to ask.
    pub model: String,
}

fn refused(field: &str, e: AiServiceError) -> CommandError {
    match e {
        AiServiceError::Secret(_) => failed(e),
        other => invalid(field, other.to_string()),
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let set_provider = {
        let services = services.clone();
        Op::new(
            "ai.settings",
            "set_provider",
            schema::<SetProviderInput>(),
            true,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                let actor = invocation.actor().clone();
                Box::pin(async move {
                    let input: SetProviderInput = parse(input)?;
                    let svc = services.get()?;
                    let key_set = input.key.as_deref().is_some_and(|k| !k.is_empty());
                    let id = input
                        .id
                        .as_deref()
                        .map(str::trim)
                        .filter(|i| !i.is_empty())
                        .unwrap_or(&input.kind)
                        .to_string();
                    let provider = ProviderConfig {
                        id: id.clone(),
                        kind: input.kind,
                        base_url: input.base_url.filter(|u| !u.trim().is_empty()),
                    };
                    if !svc.ai.uses_settings() {
                        svc.ai
                            .save_provider(provider, input.key)
                            .map_err(|e| refused("/id", e))?;
                        let events = svc.events.clone();
                        return Ok(HandlerOutput {
                            result: json!({ "id": id, "key_given": key_set }),
                            after_commit: Some(Box::new(move || {
                                events.emit(OxplowEvent::ConfigChanged)
                            })),
                            ..HandlerOutput::default()
                        });
                    }
                    // A provider is its settings (`oxplow.ai.<id>.*`), written by
                    // the one settings write path: checked, logged, undoable; its
                    // key is the keychain's and is not undone.
                    let changes = svc
                        .ai
                        .provider_changes(&provider)
                        .map_err(|e| refused("/id", e))?;
                    let mut out = super::settings_commands::write_changes(
                        &svc.config_target(),
                        &actor,
                        changes,
                        oxplow_config::settings::Level::Global,
                    )?;
                    svc.ai
                        .store_provider_key(&provider, input.key)
                        .map_err(|e| refused("/id", e))?;
                    out.result["id"] = json!(id);
                    out.result["key_given"] = json!(key_set);
                    Ok(out)
                })
            })),
        )
    };
    let remove_provider = {
        let services = services.clone();
        Op::new(
            "ai.settings",
            "remove_provider",
            schema::<ProviderInput>(),
            true,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                let actor = invocation.actor().clone();
                Box::pin(async move {
                    let input: ProviderInput = parse(input)?;
                    let svc = services.get()?;
                    if !svc.ai.uses_settings() {
                        svc.ai
                            .remove_provider(&input.id)
                            .map_err(|e| refused("/id", e))?;
                        let events = svc.events.clone();
                        return Ok(HandlerOutput {
                            result: json!({ "id": input.id }),
                            after_commit: Some(Box::new(move || {
                                events.emit(OxplowEvent::ConfigChanged)
                            })),
                            ..HandlerOutput::default()
                        });
                    }
                    let changes = svc
                        .ai
                        .removal_changes(&input.id)
                        .map_err(|e| refused("/id", e))?;
                    let mut out = super::settings_commands::write_changes(
                        &svc.config_target(),
                        &actor,
                        changes,
                        oxplow_config::settings::Level::Global,
                    )?;
                    svc.ai
                        .forget_provider_key(&input.id)
                        .map_err(|e| refused("/id", e))?;
                    out.result["id"] = json!(input.id);
                    Ok(out)
                })
            })),
        )
    };
    let set_role = {
        let services = services.clone();
        Op::new(
            "ai.settings",
            "set_role",
            schema::<SetRoleInput>(),
            true,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                let actor = invocation.actor().clone();
                Box::pin(async move {
                    let input: SetRoleInput = parse(input)?;
                    let svc = services.get()?;
                    let role = Role::parse(&input.role).map_err(|m| invalid("/role", m))?;
                    let binding =
                        match (input.provider, input.model) {
                            (Some(provider), Some(model)) => Some(RoleBinding {
                                provider,
                                model: model.trim().to_string(),
                                restrict: input.restrict.into_iter().collect(),
                            }),
                            (None, None) => None,
                            _ => return Err(invalid(
                                "/provider",
                                "give both a provider and a model, or neither to unbind the role",
                            )),
                        };
                    let bound = binding.is_some();
                    if !svc.ai.uses_settings() {
                        svc.ai
                            .set_role(&role, binding)
                            .map_err(|e| refused("/role", e))?;
                        let events = svc.events.clone();
                        return Ok(HandlerOutput {
                            result: json!({ "role": input.role, "bound": bound }),
                            after_commit: Some(Box::new(move || {
                                events.emit(OxplowEvent::ConfigChanged)
                            })),
                            ..HandlerOutput::default()
                        });
                    }
                    // A role's binding is a setting (`oxplow.ai.roles.<role>`),
                    // written by the one settings write path.
                    let change = svc
                        .ai
                        .role_change(&role, binding)
                        .map_err(|e| refused("/role", e))?;
                    let mut out = super::settings_commands::write_changes(
                        &svc.config_target(),
                        &actor,
                        vec![change],
                        oxplow_config::settings::Level::Global,
                    )?;
                    out.result["role"] = json!(input.role);
                    out.result["bound"] = json!(bound);
                    Ok(out)
                })
            })),
        )
    };
    let test_provider = Op::new(
        "ai.test",
        "test_provider",
        schema::<TestProviderInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: TestProviderInput = parse(input)?;
                let svc = services.get()?;
                let reply = svc
                    .ai
                    .test_provider(&input.id, &input.model)
                    .await
                    .map_err(|e| refused("/id", e))?;
                Ok(HandlerOutput {
                    result: json!({ "reply": reply }),
                    ..HandlerOutput::default()
                })
            })
        })),
    );
    vec![set_provider, remove_provider, set_role, test_provider]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    async fn run(
        fx: &crate::test_fixtures::ThreadFixture,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
    ) -> Result<crate::commands::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, true).await
    }

    /// A person sets a provider (its key goes to the keychain and is never
    /// read back, nor recorded), binds a role to it, can't remove it while
    /// a role uses it, unbinds the role and then removes it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_sets_providers_and_roles_and_the_key_is_not_recorded() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let me = Actor::Human;
        let out = run(
            &fx,
            &me,
            super::SET_PROVIDER,
            json!({ "id": "openrouter", "kind": "openrouter", "key": "sk-secret-123" }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["key_given"], json!(true));
        let settings = fx.svc.ai.settings().await.unwrap();
        assert_eq!(settings.providers[0].id, "openrouter");
        assert!(settings.providers[0].key_set);

        run(
            &fx,
            &me,
            super::SET_ROLE,
            json!({ "role": "summarize", "provider": "openrouter", "model": "m" }),
        )
        .await
        .unwrap();
        let err = run(
            &fx,
            &me,
            super::REMOVE_PROVIDER,
            json!({ "id": "openrouter" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");

        run(&fx, &me, super::SET_ROLE, json!({ "role": "summarize" }))
            .await
            .unwrap();
        run(
            &fx,
            &me,
            super::REMOVE_PROVIDER,
            json!({ "id": "openrouter" }),
        )
        .await
        .unwrap();
        assert!(fx.svc.ai.settings().await.unwrap().providers.is_empty());

        // A refusal that could quote the input is recorded without its
        // words when the call carried a key.
        run(
            &fx,
            &me,
            super::SET_PROVIDER,
            json!({ "id": "bad", "kind": "no-such-kind", "key": "sk-secret-123" }),
        )
        .await
        .unwrap_err();
        let recorded = fx
            .svc
            .sql
            .query_sql(
                "SELECT error FROM v_command_run WHERE command = 'oxplow.ai.set_provider' \
                 AND outcome != 'ok'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(recorded.rows.len(), 1);
        let error = format!("{:?}", recorded.rows[0][0]);
        assert!(error.contains("private input"), "{error}");
        assert!(!error.contains("sk-secret-123"), "{error}");

        // A person's page (Settings → AI as a form) runs it, and its record
        // keeps the key out too; a page an agent presses can't.
        let page = |presser: Actor| Actor::Page {
            page_id: "oxplow/ai".into(),
            on_behalf_of: Box::new(presser),
        };
        run(
            &fx,
            &page(Actor::Human),
            super::SET_PROVIDER,
            json!({ "id": "openrouter", "kind": "openrouter", "key": "sk-form-456" }),
        )
        .await
        .unwrap();
        assert!(fx
            .svc
            .ai
            .settings()
            .await
            .unwrap()
            .providers
            .iter()
            .any(|p| p.id == "openrouter" && p.key_set));
        let recorded = fx
            .svc
            .commands
            .command_run_store()
            .list_recent(5)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.command == super::SET_PROVIDER)
            .unwrap();
        let input = recorded.input.to_string();
        assert!(
            input.contains("openrouter") && !input.contains("sk-form-456"),
            "{input}"
        );
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        // A page an agent pressed may set one too.
        run(
            &fx,
            &page(agent),
            super::SET_PROVIDER,
            json!({ "id": "openrouter", "kind": "openrouter", "key": "sk-x" }),
        )
        .await
        .unwrap();
        assert!(fx
            .svc
            .ai
            .settings()
            .await
            .unwrap()
            .providers
            .iter()
            .any(|p| p.id == "openrouter"));
    }

    /// A provider and a role binding are settings written by the one write
    /// path: each logs `config.changed`, and the command undoes (the key,
    /// which is the keychain's, stays).
    #[tokio::test(flavor = "multi_thread")]
    async fn provider_and_role_writes_are_logged_and_undoable() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let me = Actor::Human;
        run(
            &fx,
            &me,
            super::SET_PROVIDER,
            json!({ "id": "openrouter", "kind": "openrouter", "key": "k" }),
        )
        .await
        .unwrap();
        let bound = run(
            &fx,
            &me,
            super::SET_ROLE,
            json!({ "role": "summarize", "provider": "openrouter", "model": "m" }),
        )
        .await
        .unwrap();
        let logged = |key: &'static str| {
            let db = fx.svc.db.clone();
            async move {
                db.read(move |c| {
                    c.query_row(
                        "SELECT count(*) FROM event_log WHERE type = 'config.changed'
                            AND json_extract(payload, '$.key') = ?1",
                        [key],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
            }
        };
        assert_eq!(
            logged("oxplow.ai.openrouter.enabled").await,
            1,
            "the provider's switch"
        );
        assert_eq!(
            logged("oxplow.ai.roles.summarize").await,
            1,
            "the role's binding"
        );
        let role = |fx: &crate::test_fixtures::ThreadFixture| {
            let svc = fx.svc.clone();
            async move {
                svc.ai.settings().await.unwrap().roles.iter().any(|r| {
                    r.role.to_string() == "summarize"
                        && r.binding
                            .as_ref()
                            .is_some_and(|b| b.provider == "openrouter")
                })
            }
        };
        assert!(role(&fx).await);
        fx.svc
            .commands
            .undo(&me, bound.command_run_id.unwrap(), false)
            .await
            .unwrap();
        assert!(!role(&fx).await, "undoing the command unbinds the role");
    }

    /// A role's binding is config, so changing it announces `ConfigChanged`
    /// like every other config write — the effective-config view refreshes
    /// the one way.
    #[tokio::test(flavor = "multi_thread")]
    async fn setting_a_role_announces_a_config_change() {
        let fx = crate::test_fixtures::services_with_thread().await;
        run(
            &fx,
            &Actor::Human,
            super::SET_PROVIDER,
            json!({ "id": "openrouter", "kind": "openrouter", "key": "k" }),
        )
        .await
        .unwrap();
        let mut rx = fx.svc.events.subscribe_ui();
        run(
            &fx,
            &Actor::Human,
            super::SET_ROLE,
            json!({ "role": "summarize", "provider": "openrouter", "model": "m" }),
        )
        .await
        .unwrap();
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            saw |= matches!(event, crate::events::OxplowEvent::ConfigChanged);
        }
        assert!(saw, "set_role emitted no ConfigChanged");
    }

    /// A provider added, replaced or removed is read from `v_ai_provider`
    /// (and `v_ai_role`'s key state), which refresh on `ConfigChanged`: so a
    /// page listing them follows the change.
    #[tokio::test(flavor = "multi_thread")]
    async fn setting_and_removing_a_provider_announce_a_config_change() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let mut rx = fx.svc.events.subscribe_ui();
        for (command, input) in [
            (
                super::SET_PROVIDER,
                json!({ "id": "openrouter", "kind": "openrouter", "key": "k" }),
            ),
            (super::REMOVE_PROVIDER, json!({ "id": "openrouter" })),
        ] {
            run(&fx, &Actor::Human, command, input).await.unwrap();
            let mut saw = false;
            while let Ok(event) = rx.try_recv() {
                saw |= matches!(event, crate::events::OxplowEvent::ConfigChanged);
            }
            assert!(saw, "{command} emitted no ConfigChanged");
        }
    }

    /// Half a binding is refused; an unknown role names the roles.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_role_binding_is_both_halves_or_neither() {
        let fx = crate::test_fixtures::services_with_thread().await;
        for input in [
            json!({ "role": "summarize", "provider": "openrouter" }),
            json!({ "role": "nonsense", "provider": "openrouter", "model": "m" }),
        ] {
            let err = run(&fx, &Actor::Human, super::SET_ROLE, input)
                .await
                .unwrap_err();
            assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        }
    }

    /// An agent may change Settings → AI: set a provider and a role, then
    /// remove the provider.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_may_change_ai_settings() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        for (name, input) in [
            (
                super::SET_PROVIDER,
                json!({ "id": "openrouter", "kind": "openrouter", "key": "k" }),
            ),
            (
                super::SET_ROLE,
                json!({ "role": "summarize", "provider": "openrouter", "model": "m" }),
            ),
            (super::SET_ROLE, json!({ "role": "summarize" })),
            (super::REMOVE_PROVIDER, json!({ "id": "openrouter" })),
        ] {
            run(&fx, &agent, name, input)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }

    /// Testing a provider that isn't there says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn testing_an_unknown_provider_is_invalid() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let err = run(
            &fx,
            &Actor::Human,
            super::TEST_PROVIDER,
            json!({ "id": "x", "model": "m" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }
}
