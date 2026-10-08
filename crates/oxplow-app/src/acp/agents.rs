//! The ACP agents a project can run: the adapters its extensions declare
//! (`acp_adapter` implementations, `Services.acp_adapters`) plus the
//! project's `acpAgents:` as instances layered over them. A project entry
//! with a declared id overrides its program and keeps its prompt mode; a new
//! name is a generic adapter (prompt ahead of the first message). Each says
//! whether it may start here (declared adapters always; a project entry
//! once a person approved it) and where its command resolves.

use std::path::Path;

use oxplow_config::{AcpAgentConfig, AcpAgentSource, OxplowConfig};
use oxplow_domain::agent::acp_adapter::SystemPromptVia;
use oxplow_domain::agent::registry::AcpAdapterRegistry;
use serde::{Deserialize, Serialize};

/// One ACP agent this project can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAgent {
    /// Its program.
    pub program: AcpAgentConfig,
    pub source: AcpAgentSource,
    /// How it gets oxplow's system prompt: its adapter's.
    pub system_prompt: SystemPromptVia,
}

/// One ACP agent, as the thread picker and Settings see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentListing {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub source: AcpAgentSource,
    /// May start on this machine: a declared adapter, or an approved
    /// project entry.
    pub approved: bool,
    /// The command's absolute path, or `None` when it isn't installed.
    pub resolved_path: Option<String>,
}

/// The declared adapters, then the project's entries (one with a declared
/// id replaces its program), in that order.
pub fn resolve(adapters: &AcpAdapterRegistry, config: &OxplowConfig) -> Vec<AcpAgent> {
    let mut out: Vec<AcpAgent> = adapters
        .all()
        .into_iter()
        .map(|a| AcpAgent {
            program: AcpAgentConfig {
                name: a.id,
                command: a.command,
                args: a.args,
                env: a.env,
            },
            source: AcpAgentSource::Declared,
            system_prompt: a.system_prompt,
        })
        .collect();
    for entry in &config.acp_agents {
        match out.iter_mut().find(|a| a.program.name == entry.name) {
            Some(agent) => {
                agent.program = entry.clone();
                agent.source = AcpAgentSource::Project;
            }
            None => out.push(AcpAgent {
                program: entry.clone(),
                source: AcpAgentSource::Project,
                system_prompt: SystemPromptVia::Prompt,
            }),
        }
    }
    out
}

/// Every ACP agent the project can pick.
pub fn list(
    approvals: &crate::exec_consent::ApprovalStore,
    project_dir: &Path,
    adapters: &AcpAdapterRegistry,
    config: &OxplowConfig,
) -> Vec<AcpAgentListing> {
    resolve(adapters, config)
        .into_iter()
        .map(|agent| AcpAgentListing {
            approved: may_start(approvals, project_dir, project_dir, &agent),
            resolved_path: resolve_command(project_dir, &agent.program.command),
            name: agent.program.name,
            command: agent.program.command,
            args: agent.program.args,
            source: agent.source,
        })
        .collect()
}

/// The named agent.
pub fn find(adapters: &AcpAdapterRegistry, config: &OxplowConfig, name: &str) -> Option<AcpAgent> {
    resolve(adapters, config)
        .into_iter()
        .find(|a| a.program.name == name)
}

/// The agent an `acp` session runs when it names none: the project's own
/// first entry, else the first declared adapter.
pub fn default_agent(adapters: &AcpAdapterRegistry, config: &OxplowConfig) -> Option<String> {
    config
        .acp_agents
        .first()
        .map(|a| a.name.clone())
        .or_else(|| adapters.all().into_iter().next().map(|a| a.id))
}

/// A declared adapter starts freely; a project entry names a program from
/// the repo, so it starts only once approved (see `exec_consent`).
pub fn may_start(
    approvals: &crate::exec_consent::ApprovalStore,
    project_dir: &Path,
    cwd: &Path,
    agent: &AcpAgent,
) -> bool {
    match agent.source {
        AcpAgentSource::Declared => true,
        AcpAgentSource::Project => {
            crate::exec_consent::may_run_acp(approvals, project_dir, cwd, &agent.program)
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::agent::acp_adapter::AcpAdapter;

    /// The adapters foundation declares, as their declarations configure
    /// them.
    fn declared() -> AcpAdapterRegistry {
        let r = AcpAdapterRegistry::default();
        r.set(vec![
            AcpAdapter::from_config(
                "claude",
                "Claude",
                &serde_json::json!({ "command": "claude-agent-acp", "systemPrompt": "meta" }),
            )
            .unwrap(),
            AcpAdapter::from_config(
                "gemini",
                "Gemini",
                &serde_json::json!({ "command": "gemini", "args": ["--acp"] }),
            )
            .unwrap(),
        ]);
        r
    }

    fn project(yaml: &str) -> (tempfile::TempDir, OxplowConfig) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/agent"), "x").unwrap();
        std::fs::write(oxplow_config::config_path(dir.path()), yaml).unwrap();
        let cfg = oxplow_config::load_project_config(dir.path()).unwrap();
        (dir, cfg)
    }

    #[test]
    fn declared_adapters_start_freely_and_project_entries_need_approval() {
        let (dir, cfg) = project("acpAgents:\n  - { name: mine, command: tools/agent }\n");
        let adapters = declared();
        let approvals = crate::exec_consent::ApprovalStore::for_tests(dir.path());
        let listed = list(&approvals, dir.path(), &adapters, &cfg);
        let names: Vec<&str> = listed.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["claude", "gemini", "mine"]);
        let get = |n: &str| listed.iter().find(|a| a.name == n).unwrap();
        assert_eq!(
            (get("claude").source, get("claude").approved),
            (AcpAgentSource::Declared, true)
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
        assert!(find(&adapters, &cfg, "mine").is_some());
        assert!(find(&adapters, &cfg, "nope").is_none());
        crate::exec_consent::approve_program(
            &approvals,
            dir.path(),
            &cfg,
            &[],
            crate::exec_consent::ProgramKind::AcpAgent,
            "mine",
            &crate::exec_consent::version_of(
                &approvals,
                dir.path(),
                &cfg,
                crate::exec_consent::ProgramKind::AcpAgent,
                "mine",
            ),
        )
        .unwrap();
        assert!(
            list(&approvals, dir.path(), &adapters, &cfg)
                .iter()
                .find(|a| a.name == "mine")
                .unwrap()
                .approved
        );
    }

    #[test]
    fn a_project_entry_overrides_a_declared_adapters_program_and_keeps_its_prompt_mode() {
        let (_dir, cfg) = project(
            "acpAgents:\n  - { name: claude, command: /opt/claude-acp, args: [--fast] }\n  - { name: mine, command: tools/agent }\n",
        );
        let adapters = declared();
        let claude = find(&adapters, &cfg, "claude").unwrap();
        assert_eq!(
            (
                claude.program.command.as_str(),
                claude.program.args.clone(),
                claude.source,
                claude.system_prompt
            ),
            (
                "/opt/claude-acp",
                vec!["--fast".to_string()],
                AcpAgentSource::Project,
                SystemPromptVia::Meta
            )
        );
        assert_eq!(
            find(&adapters, &cfg, "mine").unwrap().system_prompt,
            SystemPromptVia::Prompt
        );
        assert_eq!(
            find(&adapters, &cfg, "gemini").unwrap().source,
            AcpAgentSource::Declared
        );
    }

    /// An `acp` session that names no agent runs the project's first entry,
    /// else the first declared adapter.
    #[test]
    fn the_default_agent_is_the_projects_first_else_the_first_declared() {
        let adapters = declared();
        let (_d1, with_entry) = project("acpAgents:\n  - { name: mine, command: tools/agent }\n");
        assert_eq!(
            default_agent(&adapters, &with_entry).as_deref(),
            Some("mine")
        );
        let (_d2, bare) = project("{}\n");
        assert_eq!(default_agent(&adapters, &bare).as_deref(), Some("claude"));
        assert_eq!(default_agent(&AcpAdapterRegistry::default(), &bare), None);
    }
}
