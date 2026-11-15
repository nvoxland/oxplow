//! Settings → Agents as commands: which agent harnesses are on and in what
//! order (`oxplow.agent.set_agents`), and one harness's declared setting
//! (`oxplow.agent.set_setting`). Each checks what only the harness registry
//! knows — a key is registered, a setting is declared — then writes its
//! key through the one config write (`config_commands::change`): validated
//! against the key's schema, logged as `config.changed`, undoable, applied
//! before success is recorded. The prompt addition is one key,
//! `agentPromptAppend`, written with `oxplow.config.set`.

use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::config_commands::settings_handler;
use super::config_commands::{change, ConfigTarget, Layer};
use super::ops::Op;
use super::util::{parse, schema};

pub const SET_AGENTS: &str = "oxplow.agent.set_agents";
pub const SET_SETTING: &str = "oxplow.agent.set_setting";
pub const SET_PROMPT_APPEND: &str = "oxplow.agent.set_prompt_append";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentsInput {
    /// The enabled agent harnesses, by key, in priority order: the first
    /// is a new session's default. At least one.
    pub agents: Vec<String>,
}

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

/// `agents` as the value of the `agents` key, each registered.
fn agents_value(harnesses: &HarnessRegistry, agents: &[String]) -> Result<Value, CommandError> {
    if agents.is_empty() {
        return Err(invalid("/agents", "enable at least one agent"));
    }
    for (i, agent) in agents.iter().enumerate() {
        harnesses
            .get(agent)
            .map_err(|e| invalid(&format!("/agents/{i}"), e.to_string()))?;
    }
    Ok(json!(agents))
}

/// The `agentConfig` value with `agent`'s `key` set to `value` (or
/// removed, blank), an emptied entry dropped. An empty map remains an
/// explicit override so clearing an inherited entry does not restore it.
fn agent_config_value(
    harnesses: &HarnessRegistry,
    current: &std::collections::BTreeMap<String, Value>,
    input: SetSettingInput,
) -> Result<Value, CommandError> {
    let harness = harnesses
        .get(&input.agent)
        .map_err(|e| invalid("/agent", e.to_string()))?;
    if !harness.settings().iter().any(|s| s.key == input.key) {
        return Err(invalid(
            "/key",
            format!("agent `{}` has no setting `{}`", input.agent, input.key),
        ));
    }
    let mut all = current.clone();
    let mut entry = all.remove(&input.agent).unwrap_or_else(|| json!({}));
    let Some(fields) = entry.as_object_mut() else {
        return Err(invalid(
            "/agent",
            format!("agentConfig.{} isn't a map of settings", input.agent),
        ));
    };
    match input
        .value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        Some(v) => {
            fields.insert(input.key, Value::String(v));
        }
        None => {
            fields.remove(&input.key);
        }
    }
    if !fields.is_empty() {
        all.insert(input.agent, entry);
    }
    Ok(json!(all))
}

/// The two `agent.*` settings commands, over the project's config.
pub fn ops(target: ConfigTarget, harnesses: HarnessRegistry) -> Vec<Op> {
    let set_agents = {
        let (t, h) = (target.clone(), harnesses.clone());
        Op::new(
            "config.write",
            "set_agents",
            schema::<SetAgentsInput>(),
            true,
            settings_handler(move |actor, input| {
                let input: SetAgentsInput = parse(input)?;
                let value = agents_value(&h, &input.agents)?;
                change(&t, actor, "agents", Some(value), Layer::Project)
            }),
        )
    };
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
                let current = Layer::Project
                    .value(&t.settings()?, "agentConfig")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|error| invalid("/agent", error.to_string()))?
                    .unwrap_or_default();
                let value = agent_config_value(&harnesses, &current, input)?;
                change(&t, actor, "agentConfig", Some(value), Layer::Project)
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
    vec![set_agents, set_setting, set_prompt_append]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    fn agents_of(svc: &crate::Services) -> Vec<String> {
        crate::config_service::read_config(&svc.config).agents
    }

    /// A person sets which agents are on, in order: written as the
    /// `agents` key (logged, audited, undoable); an unregistered one is
    /// refused at its place in the list; none at all is refused; an agent
    /// can't, nor can a page it presses; the person's page can.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_sets_the_agents_in_order() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let ids: Vec<String> = svc
            .harnesses
            .all()
            .iter()
            .map(|h| h.id().to_string())
            .collect();
        assert!(ids.len() >= 2, "{ids:?}");
        let order = vec![ids[1].clone(), ids[0].clone()];
        let out = svc
            .commands
            .run(&Actor::Human, SET_AGENTS, json!({ "agents": order }), false)
            .await
            .unwrap();
        assert_eq!(agents_of(svc), order);

        for (input, field) in [
            (json!({ "agents": [ids[0], "no-such-agent"] }), "/agents/1"),
            (json!({ "agents": [] }), "/agents"),
        ] {
            let err = svc
                .commands
                .run(&Actor::Human, SET_AGENTS, input, false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{err:?}"
            );
        }
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let page = |presser: Actor| Actor::Page {
            page_id: "oxplow/agents".into(),
            on_behalf_of: Box::new(presser),
        };
        for actor in [agent.clone(), page(agent)] {
            // An agent, or a page it pressed, may set the order too.
            svc.commands
                .run(&actor, SET_AGENTS, json!({ "agents": [ids[0]] }), false)
                .await
                .unwrap();
            assert_eq!(agents_of(svc), vec![ids[0].clone()]);
        }
        svc.commands
            .run(
                &page(Actor::Human),
                SET_AGENTS,
                json!({ "agents": [ids[0]] }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(agents_of(svc), vec![ids[0].clone()]);

        // Undoing the first run puts back what was there before it.
        svc.commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .unwrap();
        assert_ne!(agents_of(svc), order);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn project_agent_setting_edits_do_not_copy_local_overrides() {
        use oxplow_config::settings::Level;
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let harness = svc
            .harnesses
            .all()
            .into_iter()
            .find(|harness| !harness.settings().is_empty())
            .unwrap();
        let agent = harness.id().to_string();
        let setting = harness.settings()[0].key.clone();
        let global = svc.layout.state_dir.join("global-config");
        let store =
            crate::settings::store(&svc.layout.project_dir, &global, svc.secrets.clone(), &[])
                .unwrap();
        let key = "oxplow.agentConfig";
        store
            .set(
                key,
                json!({(agent.clone()):{(setting.clone()):"global"}}),
                Level::Global,
            )
            .unwrap();
        store
            .set(
                key,
                json!({(agent.clone()):{(setting.clone()):"project","projectOnly":"shared"}}),
                Level::Project,
            )
            .unwrap();
        store
            .set(
                key,
                json!({(agent.clone()):{(setting.clone()):"local","localOnly":"private"}}),
                Level::Local,
            )
            .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.commands
            .run(
                &Actor::Human,
                SET_SETTING,
                json!({"agent":agent,"key":setting,"value":"edited"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            store.get(key, Some(Level::Project)).unwrap().value,
            Some(json!({(agent.clone()):{(setting.clone()):"edited","projectOnly":"shared"}}))
        );
        assert_eq!(
            store.get(key, Some(Level::Local)).unwrap().value,
            Some(json!({(agent.clone()):{(setting.clone()):"local","localOnly":"private"}}))
        );
        store.unset(key, Level::Project).unwrap();
        svc.commands
            .run(
                &Actor::Human,
                SET_SETTING,
                json!({"agent":agent,"key":setting,"value":""}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            store.get(key, Some(Level::Project)).unwrap().value,
            Some(json!({}))
        );
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
            .find(|h| !h.settings().is_empty())
            .expect("a harness declares a setting");
        let (agent, key) = (harness.id().to_string(), harness.settings()[0].key.clone());
        let config = || crate::config_service::read_config(&svc.config).agent_config;
        svc.commands
            .run(
                &Actor::Human,
                SET_SETTING,
                json!({ "agent": agent, "key": key, "value": " v1 " }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(config()[&agent][&key], json!("v1"));
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
