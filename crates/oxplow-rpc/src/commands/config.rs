//! Cores for the `config` command module.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::commands::config_commands::{SET, UNSET};
use oxplow_app::config_service::read_config;
use oxplow_app::Services;
use oxplow_config::{AgentKind, GeneratedConfig, OxplowConfig};
use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::error::IpcError;

pub async fn get_config(svc: &Services) -> Result<OxplowConfig, IpcError> {
    Ok(read_config(&svc.config))
}

/// Set `key` (or unset it, with `None`) as the person, through
/// `oxplow.config.set` / `oxplow.config.unset`: validated, audited, logged as
/// `config.changed` and undoable, like any config change. The person's
/// own action is their confirmation for a human-only key. Returns the
/// config as it is afterwards.
pub(crate) async fn set_key(
    svc: &Services,
    key: &str,
    value: Option<Value>,
) -> Result<OxplowConfig, IpcError> {
    let (command, input) = match value {
        Some(value) => (SET, json!({ "key": key, "value": value })),
        None => (UNSET, json!({ "key": key })),
    };
    svc.commands
        .run(&Actor::Human, command, input, true)
        .await?;
    Ok(read_config(&svc.config))
}

fn value_of<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("config values serialize")
}

pub async fn set_agent_prompt_append(
    svc: &Services,
    text: String,
) -> Result<OxplowConfig, IpcError> {
    let value = (!text.trim().is_empty()).then(|| Value::String(text));
    set_key(svc, "agentPromptAppend", value).await
}

pub async fn set_agents(svc: &Services, agents: Vec<AgentKind>) -> Result<OxplowConfig, IpcError> {
    set_key(svc, "agents", Some(value_of(&agents))).await
}

/// Set (or clear, with `None`/blank) the launch-model override for one
/// agent — `agentModels.<agent>` in .oxplow/project.yaml. Only opencode consumes
/// the override today.
pub async fn set_agent_model(
    svc: &Services,
    agent: AgentKind,
    model: Option<String>,
) -> Result<OxplowConfig, IpcError> {
    let mut models = read_config(&svc.config).agent_models;
    match model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
    {
        Some(m) => {
            models.insert(agent, m);
        }
        None => {
            models.remove(&agent);
        }
    }
    let value = (!models.is_empty()).then(|| value_of(&models));
    set_key(svc, "agentModels", value).await
}

/// The generated-file include/exclude lists. The snapshot captures pick up
/// the new filter from the `config.changed` event (`config_reactors`), the
/// same way they do when an agent sets the key.
pub async fn set_generated(
    svc: &Services,
    generated: GeneratedConfig,
) -> Result<OxplowConfig, IpcError> {
    let value = (generated != GeneratedConfig::default()).then(|| value_of(&generated));
    set_key(svc, "generated", value).await
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceContext {
    pub project_dir: String,
    pub default_branch: Option<String>,
    pub vcs_enabled: bool,
}

pub async fn get_workspace_context(svc: &Services) -> Result<WorkspaceContext, IpcError> {
    let project = svc.layout.project_dir.clone();
    let project_str = project.to_string_lossy().into_owned();
    let vcs_enabled = svc.vcs.detect(&project).await.is_some();
    let default_branch = if vcs_enabled {
        svc.vcs
            .branches(&project)
            .await
            .ok()
            .and_then(|all| all.into_iter().find(|b| b.is_default && b.remote.is_none()))
            .map(|b| b.name)
    } else {
        None
    };
    Ok(WorkspaceContext {
        project_dir: project_str,
        default_branch,
        vcs_enabled,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_support::services;

    #[tokio::test]
    async fn get_config_dispatches_with_no_args() {
        let (svc, _dir) = services();
        let out = crate::dispatch("get_config", json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_object(), "expected a config object, got {out}");
    }

    /// `(actor, key)` of each successful `oxplow.config.set` / `oxplow.config.unset`, oldest first.
    async fn audited_config_sets(svc: &crate::RpcContext) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> =
            oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
                .list_recent(100)
                .await
                .unwrap()
                .into_iter()
                .filter(|a| {
                    (a.command == "oxplow.config.set" || a.command == "oxplow.config.unset")
                        && a.error.is_none()
                })
                .map(|a| {
                    (
                        serde_json::to_value(a.actor_kind)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_string(),
                        a.input["key"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
        rows.reverse();
        rows
    }

    /// tsk515: every settings write is the person's `oxplow.config.set` — audited,
    /// logged as `config.changed`, undoable — and the file says what the
    /// returned config says.
    #[tokio::test]
    async fn settings_writes_are_the_persons_config_commands() {
        let (svc, dir) = services();
        for (command, args) in [
            ("set_agents", json!({ "agents": ["claude", "codex"] })),
            ("set_agent_prompt_append", json!({ "text": "Be brief." })),
            (
                "set_agent_model",
                json!({ "agent": "opencode", "model": "m1" }),
            ),
            (
                "set_generated",
                json!({ "generated": { "exclude": ["dist"], "include": [] } }),
            ),
            (
                "set_extension_enabled",
                json!({ "name": "oxplow-bundled", "enabled": false }),
            ),
        ] {
            crate::dispatch(command, args, &svc)
                .await
                .unwrap_or_else(|e| panic!("{command}: {}", e.message));
        }
        let keys: Vec<String> = audited_config_sets(&svc)
            .await
            .into_iter()
            .map(|(actor, key)| {
                assert_eq!(actor, "human");
                key
            })
            .collect();
        assert_eq!(
            keys,
            [
                "agents",
                "agentPromptAppend",
                "agentModels",
                "generated",
                "extensions",
            ]
        );
        let file = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        for expected in ["codex", "Be brief.", "m1", "dist", "oxplow-bundled"] {
            assert!(file.contains(expected), "{expected} missing from:\n{file}");
        }
        // Clearing a value unsets its key.
        let out = crate::dispatch(
            "set_agent_model",
            json!({ "agent": "opencode", "model": null }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["agentModels"], json!({}));
        assert_eq!(
            audited_config_sets(&svc).await.last().unwrap().1,
            "agentModels"
        );
    }
}
