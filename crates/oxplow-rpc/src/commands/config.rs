//! Cores for the `config` command module.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::{mutate_config, read_config};
use oxplow_app::Services;
use oxplow_config::{AgentKind, OxplowConfig};

use crate::error::IpcError;

pub async fn get_config(svc: &Services) -> Result<OxplowConfig, IpcError> {
    Ok(read_config(&svc.config))
}

pub async fn set_agent_prompt_append(
    svc: &Services,
    text: String,
) -> Result<OxplowConfig, IpcError> {
    let project = svc.layout.project_dir.clone();
    mutate_config(&svc.config, &project, |c| c.agent_prompt_append = text)
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn set_agents(svc: &Services, agents: Vec<AgentKind>) -> Result<OxplowConfig, IpcError> {
    if agents.is_empty() {
        return Err(IpcError::invalid(
            "at least one agent must be enabled for the project",
        ));
    }
    for (idx, agent) in agents.iter().enumerate() {
        if agents[..idx].contains(agent) {
            return Err(IpcError::invalid(format!(
                "agent {} is listed more than once",
                agent.as_str()
            )));
        }
    }
    let project = svc.layout.project_dir.clone();
    mutate_config(&svc.config, &project, |c| c.agents = agents)
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn set_snapshot_retention_days(
    svc: &Services,
    days: u32,
) -> Result<OxplowConfig, IpcError> {
    let project = svc.layout.project_dir.clone();
    mutate_config(&svc.config, &project, |c| c.snapshot_retention_days = days)
        .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn set_snapshot_max_file_bytes(
    svc: &Services,
    bytes: u64,
) -> Result<OxplowConfig, IpcError> {
    let project = svc.layout.project_dir.clone();
    mutate_config(&svc.config, &project, |c| c.snapshot_max_file_bytes = bytes)
        .map_err(|e| IpcError::internal(e.to_string()))
}

/// Set (or clear, with `None`/blank) the launch-model override for one
/// agent — `agentModels.<agent>` in oxplow.yaml. Only opencode consumes
/// the override today.
pub async fn set_agent_model(
    svc: &Services,
    agent: AgentKind,
    model: Option<String>,
) -> Result<OxplowConfig, IpcError> {
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let project = svc.layout.project_dir.clone();
    mutate_config(&svc.config, &project, |c| match model {
        Some(m) => {
            c.agent_models.insert(agent, m);
        }
        None => {
            c.agent_models.remove(&agent);
        }
    })
    .map_err(|e| IpcError::internal(e.to_string()))
}

pub async fn set_generated(
    svc: &Services,
    generated: oxplow_config::GeneratedConfig,
) -> Result<OxplowConfig, IpcError> {
    let project = svc.layout.project_dir.clone();
    let updated = mutate_config(&svc.config, &project, |c| c.generated = generated.clone())
        .map_err(|e| IpcError::internal(e.to_string()))?;
    // Push the new filter into every live snapshot capture so the
    // include/exclude change takes effect immediately — without this
    // the toggle would silently no-op until the app restarts.
    svc.snapshot_captures
        .set_workspace_filter(oxplow_fs_watch::WorkspaceFilter::for_project(
            &project,
            &updated.generated.exclude,
            &updated.generated.include,
        ));
    Ok(updated)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceContext {
    pub project_dir: String,
    pub default_branch: Option<String>,
    pub is_git_repo: bool,
}

pub async fn get_workspace_context(svc: &Services) -> Result<WorkspaceContext, IpcError> {
    let project = svc.layout.project_dir.clone();
    let project_str = project.to_string_lossy().into_owned();
    let is_git_repo = tokio::task::spawn_blocking(move || oxplow_git::is_git_repo(&project))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let default_branch = if is_git_repo {
        svc.git.detect_default_branch().await
    } else {
        None
    };
    Ok(WorkspaceContext {
        project_dir: project_str,
        default_branch,
        is_git_repo,
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

    #[tokio::test]
    async fn set_snapshot_retention_days_round_trips_arg() {
        let (svc, _dir) = services();
        let out = crate::dispatch("set_snapshot_retention_days", json!({ "days": 7 }), &svc)
            .await
            .unwrap();
        assert_eq!(out.get("snapshotRetentionDays"), Some(&json!(7)));
    }
}
