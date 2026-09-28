//! The ACP agents a project can run: oxplow's presets plus the project's
//! `acpAgents:`, each with whether it may start here (presets always; a
//! project entry once a person approved it) and where its command
//! resolves.

use std::path::Path;

use oxplow_config::{AcpAgentConfig, AcpAgentSource, OxplowConfig};
use serde::{Deserialize, Serialize};

/// One ACP agent, as the thread picker and Settings see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentListing {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub source: AcpAgentSource,
    /// May start on this machine: a preset, or an approved project entry.
    pub approved: bool,
    /// The command's absolute path, or `None` when it isn't installed.
    pub resolved_path: Option<String>,
}

/// Every ACP agent the project can pick.
pub fn list(
    approvals: &crate::exec_consent::ApprovalStore,
    project_dir: &Path,
    config: &OxplowConfig,
) -> Vec<AcpAgentListing> {
    oxplow_config::resolve_acp_agents(&config.acp_agents)
        .into_iter()
        .map(|(agent, source)| AcpAgentListing {
            approved: may_start(approvals, project_dir, project_dir, &agent, source),
            resolved_path: resolve_command(project_dir, &agent.command),
            name: agent.name,
            command: agent.command,
            args: agent.args,
            source,
        })
        .collect()
}

/// The named agent and where it came from.
pub fn find(config: &OxplowConfig, name: &str) -> Option<(AcpAgentConfig, AcpAgentSource)> {
    oxplow_config::resolve_acp_agents(&config.acp_agents)
        .into_iter()
        .find(|(a, _)| a.name == name)
}

/// Presets start freely; a project entry names a program from the repo,
/// so it starts only once approved (see `exec_consent`).
pub fn may_start(
    approvals: &crate::exec_consent::ApprovalStore,
    project_dir: &Path,
    cwd: &Path,
    agent: &AcpAgentConfig,
    source: AcpAgentSource,
) -> bool {
    match source {
        AcpAgentSource::Preset => true,
        AcpAgentSource::Project => {
            crate::exec_consent::may_run_acp(approvals, project_dir, cwd, agent)
        }
    }
}

/// Absolute path of `command`: a path in the project, or a program on
/// PATH / the well-known install dirs.
pub fn resolve_command(project_dir: &Path, command: &str) -> Option<String> {
    if command.contains('/') {
        let p = project_dir.join(command);
        return p.is_file().then(|| p.to_string_lossy().into_owned());
    }
    crate::agent_path::resolve_program(command)
}

/// The Claude adapter takes oxplow's system prompt in
/// `_meta.systemPrompt.append` on `session/new`; other agents get it
/// ahead of the first prompt.
pub fn system_prompt_via_meta(agent: &AcpAgentConfig) -> bool {
    Path::new(&agent.command)
        .file_name()
        .is_some_and(|n| n == "claude-agent-acp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_start_freely_and_project_entries_need_approval() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/agent"), "x").unwrap();
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "acpAgents:\n  - { name: mine, command: tools/agent }\n",
        )
        .unwrap();
        let cfg = oxplow_config::load_project_config(dir.path()).unwrap();
        let approvals = crate::exec_consent::ApprovalStore::for_tests(dir.path());
        let listed = list(&approvals, dir.path(), &cfg);
        let get = |n: &str| listed.iter().find(|a| a.name == n).unwrap();
        assert_eq!(
            (get("claude").source, get("claude").approved),
            (AcpAgentSource::Preset, true)
        );
        let mine = get("mine");
        assert_eq!(
            (mine.source, mine.approved),
            (AcpAgentSource::Project, false)
        );
        assert!(mine
            .resolved_path
            .as_deref()
            .unwrap()
            .ends_with("tools/agent"));
        assert!(find(&cfg, "mine").is_some() && find(&cfg, "nope").is_none());
        crate::exec_consent::approve_program(
            &approvals,
            dir.path(),
            &cfg,
            crate::exec_consent::ProgramKind::AcpAgent,
            "mine",
        )
        .unwrap();
        assert!(
            list(&approvals, dir.path(), &cfg)
                .iter()
                .find(|a| a.name == "mine")
                .unwrap()
                .approved
        );
    }
}
