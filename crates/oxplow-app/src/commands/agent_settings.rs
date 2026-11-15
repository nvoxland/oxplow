//! Harness preference adapters write each declared scalar through the shared settings service.

use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::config_commands::settings_handler;
use super::config_commands::{change, ConfigTarget, Layer};
use super::ops::Op;
use super::util::{parse, schema};

pub const SET_SETTING: &str = "oxplow.agent.set_setting";
pub const SET_PROMPT_APPEND: &str = "oxplow.agent.set_prompt_append";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSettingInput {
    /// The harness (`claude`, `codex`, `opencode`).
    pub agent: String,
    /// One of the settings it declares (`model`, `effort`).
    pub key: String,
    /// Its value; absent or blank clears it.
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromptAppendInput {
    /// Text appended to every agent's system prompt (`agentPromptAppend`);
    /// blank removes it.
    #[serde(default)]
    pub text: String,
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message: message.into(),
    }
}

/// The two `agent.*` settings commands, over the project's config.
pub fn ops(target: ConfigTarget, harnesses: HarnessRegistry) -> Vec<Op> {
    let prompt_target = target.clone();
    let set_setting = {
        let t = target;
        Op::new(
            "config.write",
            "set_agent_setting",
            schema::<SetSettingInput>(),
            true,
            settings_handler(move |actor, input| {
                let input: SetSettingInput = parse(input)?;
                let harness = harnesses
                    .get(&input.agent)
                    .map_err(|e| invalid("/agent", e.to_string()))?;
                if !harness.settings().iter().any(|s| s.key == input.key) {
                    return Err(invalid(
                        "/key",
                        format!("agent `{}` has no setting `{}`", input.agent, input.key),
                    ));
                }
                let key = format!(
                    "oxplow.agents.{}.{}",
                    input.agent.replace('/', "."),
                    oxplow_config::settings::key_segment(&input.key)
                );
                let value = input
                    .value
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .map(Value::String);
                change(&t, actor, &key, value, Layer::Project)
            }),
        )
    };
    let set_prompt_append = Op::new(
        "config.write",
        "set_prompt_append",
        schema::<SetPromptAppendInput>(),
        true,
        settings_handler(move |actor, input| {
            let input: SetPromptAppendInput = parse(input)?;
            let value = (!input.text.trim().is_empty()).then(|| json!(input.text.trim()));
            change(
                &prompt_target,
                actor,
                "agentPromptAppend",
                value,
                Layer::Project,
            )
        }),
    );
    vec![set_setting, set_prompt_append]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    #[tokio::test(flavor = "multi_thread")]
    async fn independent_agent_enablement_allows_disabling_every_harness() {
        let fx = crate::test_fixtures::services_with_thread().await;
        for agent in fx.svc.harnesses.names() {
            fx.svc.commands.run(&Actor::Human, "oxplow.config.set", json!({"key":format!("oxplow.agents.{agent}.enabled"),"value":false,"level":"project"}), false).await.unwrap();
        }
        assert!(crate::config_service::read_config(&fx.svc.config)
            .agent_enabled
            .values()
            .all(|enabled| !enabled));
        let listed = crate::harnesses::listing(
            &fx.svc.harnesses,
            &fx.svc.capabilities,
            &crate::config_service::read_config(&fx.svc.config).agent_enabled,
        );
        assert!(!listed.is_empty());
        assert!(listed.iter().all(|h| !h.enabled));
        assert!(listed
            .windows(2)
            .all(|pair| pair[0].title.to_lowercase() <= pair[1].title.to_lowercase()));
    }

    /// The prompt addition is written as `agentPromptAppend` (blank removes
    /// it), by a person, an agent or a page for either.
    #[tokio::test(flavor = "multi_thread")]
    async fn anyone_sets_the_prompt_addition() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let prompt = || crate::config_service::read_config(&svc.config).agent_prompt_append;
        svc.commands
            .run(
                &Actor::Human,
                SET_PROMPT_APPEND,
                json!({ "text": "  Prefer TDD.  " }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(prompt(), "Prefer TDD.");
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let page = Actor::Page {
            page_id: "oxplow/agents-prompt".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        for actor in [agent, page] {
            svc.commands
                .run(&actor, SET_PROMPT_APPEND, json!({ "text": "x" }), true)
                .await
                .unwrap();
            assert_eq!(prompt(), "x");
        }
        svc.commands
            .run(
                &Actor::Human,
                SET_PROMPT_APPEND,
                json!({ "text": " " }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(prompt(), "");
    }

    /// A person sets one of a harness's declared settings; a blank value
    /// clears it and an emptied entry goes; an undeclared setting or an
    /// unregistered agent is refused at its field.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_sets_an_agents_declared_setting() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let harness = svc
            .harnesses
            .all()
            .into_iter()
            .find(|h| h.settings().iter().any(|setting| setting.key == "model"))
            .expect("a harness declares a setting");
        let (agent, key) = (harness.id().to_string(), "model".to_string());
        let config = || crate::config_service::read_config(&svc.config).agent_config;
        svc.commands
            .run(
                &Actor::Human,
                SET_SETTING,
                json!({ "agent": agent, "key": key, "value": " opus " }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(config()[&agent][&key], json!("opus"));
        svc.commands
            .run(
                &Actor::Human,
                SET_SETTING,
                json!({ "agent": agent, "key": key, "value": "" }),
                false,
            )
            .await
            .unwrap();
        assert!(!config().contains_key(&agent), "{:?}", config());

        for (input, field) in [
            (
                json!({ "agent": agent, "key": "no-such-setting", "value": "x" }),
                "/key",
            ),
            (
                json!({ "agent": "no-such-agent", "key": key, "value": "x" }),
                "/agent",
            ),
        ] {
            let err = svc
                .commands
                .run(&Actor::Human, SET_SETTING, input, false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{err:?}"
            );
        }
    }
}
