//! Settings → Agents as commands: which agent harnesses are on and in what
//! order (`oxplow.agent.set_agents`), and one harness's declared setting
//! (`oxplow.agent.set_setting`). Each checks what only the harness registry
//! knows — a key is registered, a setting is declared — then writes its
//! key through the one config write (`config_commands::change`): validated
//! against the key's schema, logged as `config.changed`, undoable, applied
//! after commit. A person's or their own lens's (`Invokers::NO_AGENT`):
//! which agents run, and how, is theirs. The prompt addition is one key,
//! `agentPromptAppend`, written with `oxplow.config.set`.

use std::sync::Arc;

use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::{CommandError, Invokers};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::config_commands::{change, ConfigTarget, Layer};
use super::ops::Op;
use super::util::{parse, schema};
use super::{Handler, TxCtx};

pub const SET_AGENTS: &str = "oxplow.agent.set_agents";
pub const SET_SETTING: &str = "oxplow.agent.set_setting";

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
/// removed, blank), an emptied entry dropped; `None` when nothing is left.
fn agent_config_value(
    harnesses: &HarnessRegistry,
    current: &std::collections::BTreeMap<String, Value>,
    input: SetSettingInput,
) -> Result<Option<Value>, CommandError> {
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
    Ok((!all.is_empty()).then(|| json!(all)))
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
            Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
                let input: SetAgentsInput = parse(input)?;
                let value = agents_value(&h, &input.agents)?;
                change(&t, ctx.actor(), "agents", Some(value), Layer::Project)
            })),
        )
        .open_to(Invokers::NO_AGENT)
    };
    let set_setting = {
        let t = target;
        Op::new(
            "config.write",
            "set_agent_setting",
            schema::<SetSettingInput>(),
            true,
            Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
                let input: SetSettingInput = parse(input)?;
                let current = t
                    .config
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .agent_config
                    .clone();
                let value = agent_config_value(&harnesses, &current, input)?;
                change(&t, ctx.actor(), "agentConfig", value, Layer::Project)
            })),
        )
        .open_to(Invokers::NO_AGENT)
    };
    vec![set_agents, set_setting]
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
    /// can't, nor can a lens it presses; the person's lens can.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_sets_the_agents_in_order() {
        let fx = crate::test_fixtures::services_with_effort().await;
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
        let lens = |presser: Actor| Actor::Lens {
            lens_id: "oxplow-foundation/agents".into(),
            on_behalf_of: Box::new(presser),
        };
        for actor in [agent.clone(), lens(agent)] {
            let err = svc
                .commands
                .run(&actor, SET_AGENTS, json!({ "agents": [ids[0]] }), true)
                .await
                .unwrap_err();
            assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        }
        svc.commands
            .run(
                &lens(Actor::Human),
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

    /// A person sets one of a harness's declared settings; a blank value
    /// clears it and an emptied entry goes; an undeclared setting or an
    /// unregistered agent is refused at its field.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_sets_an_agents_declared_setting() {
        let fx = crate::test_fixtures::services_with_effort().await;
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
