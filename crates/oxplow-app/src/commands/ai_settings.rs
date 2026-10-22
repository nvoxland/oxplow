//! Settings → AI as commands: `oxplow.ai.set_provider` adds or replaces a
//! provider (and stores its key in the keychain), `remove_provider` takes
//! one away, `set_role` binds a role to a provider and model (or unbinds
//! it) and `test_provider` makes one small call to check a provider. They
//! are a person's alone (`Invokers::NO_AGENT`): the keys and where the
//! person's money goes are theirs to set. Their own lens (Settings → AI as
//! a form) runs them; an agent can't, and neither can a lens an agent
//! presses (the bus admits a lens only when its presser is admitted). `set_provider` keeps the `key` out of the
//! audit (`with_unrecorded`: only that one was given is recorded).
//! `External`: the settings are the user-global `ai.yaml` and the OS
//! keychain, outside oxplow's database. Reading them is
//! `v_ai_provider` / `v_ai_role`.

use std::sync::Arc;

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_domain::ai::AiRestriction;
use oxplow_domain::{CommandError, Invokers};
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
    /// Your name for it (`anthropic`, `local-ollama`); also its keychain entry.
    pub id: String,
    /// What it is: a declared AI provider kind (`anthropic`, `openai`,
    /// `openai_compatible`, `openrouter`).
    pub kind: String,
    /// Its API base: required for a kind with no default, an override for the others.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Its API key, stored in the OS keychain and never read back. Left
    /// out (or empty), the stored one stays.
    #[serde(default)]
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
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: SetProviderInput = parse(input)?;
                    let svc = services.get()?;
                    let key_set = input.key.as_deref().is_some_and(|k| !k.is_empty());
                    let id = input.id.trim().to_string();
                    svc.ai
                        .save_provider(
                            ProviderConfig {
                                id: id.clone(),
                                kind: input.kind,
                                base_url: input.base_url.filter(|u| !u.trim().is_empty()),
                            },
                            input.key,
                        )
                        .map_err(|e| refused("/id", e))?;
                    Ok(HandlerOutput {
                        result: json!({ "id": id, "key_given": key_set }),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .open_to(Invokers::NO_AGENT)
        .with_unrecorded(&["key"])
    };
    let remove_provider = {
        let services = services.clone();
        Op::new(
            "ai.settings",
            "remove_provider",
            schema::<ProviderInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: ProviderInput = parse(input)?;
                    let svc = services.get()?;
                    svc.ai
                        .remove_provider(&input.id)
                        .map_err(|e| refused("/id", e))?;
                    Ok(HandlerOutput {
                        result: json!({ "id": input.id }),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .open_to(Invokers::NO_AGENT)
    };
    let set_role = {
        let services = services.clone();
        Op::new(
            "ai.settings",
            "set_role",
            schema::<SetRoleInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
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
                    svc.ai
                        .set_role(&role, binding)
                        .map_err(|e| refused("/role", e))?;
                    // A role's binding is config: the effective-config view
                    // refreshes the one way.
                    let events = svc.events.clone();
                    Ok(HandlerOutput {
                        result: json!({ "role": input.role, "bound": bound }),
                        after_commit: Some(Box::new(move || {
                            events.emit(OxplowEvent::ConfigChanged)
                        })),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .open_to(Invokers::NO_AGENT)
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
    )
    .open_to(Invokers::NO_AGENT);
    vec![set_provider, remove_provider, set_role, test_provider]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    async fn run(
        fx: &crate::test_fixtures::EffortFixture,
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
        let fx = crate::test_fixtures::services_with_effort().await;
        let me = Actor::Human;
        let out = run(
            &fx,
            &me,
            super::SET_PROVIDER,
            json!({ "id": "or", "kind": "openrouter", "key": "sk-secret-123" }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["key_given"], json!(true));
        let settings = fx.svc.ai.settings().await.unwrap();
        assert_eq!(settings.providers[0].id, "or");
        assert!(settings.providers[0].key_set);

        run(
            &fx,
            &me,
            super::SET_ROLE,
            json!({ "role": "summarize", "provider": "or", "model": "m" }),
        )
        .await
        .unwrap();
        let err = run(&fx, &me, super::REMOVE_PROVIDER, json!({ "id": "or" }))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");

        run(&fx, &me, super::SET_ROLE, json!({ "role": "summarize" }))
            .await
            .unwrap();
        run(&fx, &me, super::REMOVE_PROVIDER, json!({ "id": "or" }))
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
        let audited = fx
            .svc
            .sql
            .query_sql(
                "SELECT error FROM v_command_audit WHERE command = 'oxplow.ai.set_provider' \
                 AND outcome != 'ok'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(audited.rows.len(), 1);
        let error = format!("{:?}", audited.rows[0][0]);
        assert!(error.contains("private input"), "{error}");
        assert!(!error.contains("sk-secret-123"), "{error}");

        // A person's lens (Settings → AI as a form) runs it, and its record
        // keeps the key out too; a lens an agent presses can't.
        let lens = |presser: Actor| Actor::Lens {
            lens_id: "oxplow-foundation/ai".into(),
            on_behalf_of: Box::new(presser),
        };
        run(
            &fx,
            &lens(Actor::Human),
            super::SET_PROVIDER,
            json!({ "id": "viaform", "kind": "openrouter", "key": "sk-form-456" }),
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
            .any(|p| p.id == "viaform" && p.key_set));
        let recorded = fx
            .svc
            .commands
            .audit_store()
            .list_recent(5)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.command == super::SET_PROVIDER)
            .unwrap();
        let input = recorded.input.to_string();
        assert!(
            input.contains("viaform") && !input.contains("sk-form-456"),
            "{input}"
        );
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = run(
            &fx,
            &lens(agent),
            super::SET_PROVIDER,
            json!({ "id": "byagent", "kind": "openrouter", "key": "sk-x" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert!(!fx
            .svc
            .ai
            .settings()
            .await
            .unwrap()
            .providers
            .iter()
            .any(|p| p.id == "byagent"));
    }

    /// A role's binding is config, so changing it announces `ConfigChanged`
    /// like every other config write — the effective-config view refreshes
    /// the one way.
    #[tokio::test(flavor = "multi_thread")]
    async fn setting_a_role_announces_a_config_change() {
        let fx = crate::test_fixtures::services_with_effort().await;
        run(
            &fx,
            &Actor::Human,
            super::SET_PROVIDER,
            json!({ "id": "or", "kind": "openrouter", "key": "k" }),
        )
        .await
        .unwrap();
        let mut rx = fx.svc.events.subscribe_ui();
        run(
            &fx,
            &Actor::Human,
            super::SET_ROLE,
            json!({ "role": "summarize", "provider": "or", "model": "m" }),
        )
        .await
        .unwrap();
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            saw |= matches!(event, crate::events::OxplowEvent::ConfigChanged);
        }
        assert!(saw, "set_role emitted no ConfigChanged");
    }

    /// Half a binding is refused; an unknown role names the roles.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_role_binding_is_both_halves_or_neither() {
        let fx = crate::test_fixtures::services_with_effort().await;
        for input in [
            json!({ "role": "summarize", "provider": "or" }),
            json!({ "role": "nonsense", "provider": "or", "model": "m" }),
        ] {
            let err = run(&fx, &Actor::Human, super::SET_ROLE, input)
                .await
                .unwrap_err();
            assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        }
    }

    /// Settings → AI is a person's alone: not an agent's.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_may_not_touch_ai_settings() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        for (name, input) in [
            (
                super::SET_PROVIDER,
                json!({ "id": "or", "kind": "openrouter", "key": "k" }),
            ),
            (super::REMOVE_PROVIDER, json!({ "id": "or" })),
            (super::SET_ROLE, json!({ "role": "summarize" })),
            (super::TEST_PROVIDER, json!({ "id": "or", "model": "m" })),
        ] {
            let err = run(&fx, &agent, name, input).await.unwrap_err();
            assert!(
                matches!(
                    err,
                    CommandError::Denied { .. } | CommandError::Unknown { .. }
                ),
                "{name}: {err:?}"
            );
        }
    }

    /// Testing a provider that isn't there says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn testing_an_unknown_provider_is_invalid() {
        let fx = crate::test_fixtures::services_with_effort().await;
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
