//! Cores for the `config` command module.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::read_config;
use oxplow_app::Services;
use oxplow_config::OxplowConfig;

use crate::error::IpcError;

pub async fn get_config(svc: &Services) -> Result<OxplowConfig, IpcError> {
    Ok(read_config(&svc.config))
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
        let (svc, _dir) = services().await;
        let out = crate::dispatch("get_config", json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_object(), "expected a config object, got {out}");
    }
}
