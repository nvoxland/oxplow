//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` registered under the key its declaration gives — the key
//! an agent session's `harness` names. Many are registered at once, the way
//! collectors are.

mod acp;
mod claude;
mod codex;
mod opencode;
mod shared;

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::HarnessRegistry;

use crate::capabilities::{Implementation, Source};

/// The harness a built-in `entry` is, registered under `id`.
pub fn built_in(entry: &str, id: &str, title: &str) -> Option<Arc<dyn AgentHarness>> {
    let named = Named {
        id: id.into(),
        title: title.into(),
    };
    Some(match entry {
        "oxplow:claude-code" => Arc::new(claude::Claude(named)),
        "oxplow:codex-cli" => Arc::new(codex::Codex(named)),
        "oxplow:opencode" => Arc::new(opencode::Opencode(named)),
        "oxplow:acp" => Arc::new(acp::Acp(named)),
        _ => return None,
    })
}

/// Register the harnesses `declared` names (the project's extensions'
/// `implementations:`), and unregister the ones it no longer does.
pub fn register_built_ins(registry: &HarnessRegistry, declared: &[Implementation]) {
    let harnesses: Vec<Arc<dyn AgentHarness>> = declared
        .iter()
        .filter(|i| i.capability == "agent_harness")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => built_in(entry, &i.id, &i.title),
            _ => None,
        })
        .collect();
    for gone in registry
        .names()
        .into_iter()
        .filter(|n| !harnesses.iter().any(|h| h.id() == n))
    {
        registry.unregister(&gone);
    }
    for h in harnesses {
        registry.register(h);
    }
}

/// A harness's key and title, as its declaration gives them.
struct Named {
    id: String,
    title: String,
}

/// Launching a harness in a scratch project, for its tests.
#[cfg(test)]
pub(crate) mod test_launch {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use oxplow_domain::agent::harness::{AgentHarness, Endpoints, Launch, LaunchInput, SessionIds};
    use oxplow_domain::{AgentSessionId, StreamId, ThreadId};

    pub struct Launched {
        pub launch: Launch,
        pub project: PathBuf,
        _dir: tempfile::TempDir,
    }

    pub fn harness(entry: &str, id: &str) -> Arc<dyn AgentHarness> {
        super::built_in(entry, id, id).expect("a built-in harness")
    }

    /// `h` launched for session ses3 (thread thr2, stream str1) in a scratch
    /// project, its programs resolved under `/opt/agents`.
    pub fn launch_in(
        h: &dyn AgentHarness,
        resume: Option<&str>,
        system_prompt: Option<&str>,
        config: &serde_json::Value,
    ) -> Launched {
        launch_full(h, resume, system_prompt, config, None)
    }

    /// As [`launch_in`], with a home `setup` prepares (given it and the
    /// workspace).
    pub fn launch_with_home(
        h: &dyn AgentHarness,
        resume: &str,
        setup: impl FnOnce(&Path, &str),
    ) -> Launched {
        let home = tempfile::tempdir().unwrap();
        let launched = {
            let dir = tempfile::tempdir().unwrap();
            let workspace = dir.path().to_string_lossy().into_owned();
            setup(home.path(), &workspace);
            run(
                h,
                Some(resume),
                None,
                &serde_json::json!({}),
                Some(home.path()),
                dir,
            )
        };
        drop(home);
        launched
    }

    fn launch_full(
        h: &dyn AgentHarness,
        resume: Option<&str>,
        system_prompt: Option<&str>,
        config: &serde_json::Value,
        home: Option<&Path>,
    ) -> Launched {
        run(
            h,
            resume,
            system_prompt,
            config,
            home,
            tempfile::tempdir().unwrap(),
        )
    }

    fn run(
        h: &dyn AgentHarness,
        resume: Option<&str>,
        system_prompt: Option<&str>,
        config: &serde_json::Value,
        home: Option<&Path>,
        dir: tempfile::TempDir,
    ) -> Launched {
        let project = dir.path().to_path_buf();
        let endpoints = Endpoints {
            hook_base_url: "http://127.0.0.1:9/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:9/mcp".into(),
            otlp_base_url: "http://127.0.0.1:9".into(),
            hook_token: "tok".into(),
        };
        let identity = vec![
            ("OXPLOW_THREAD_ID".to_string(), "thr2".to_string()),
            ("OXPLOW_SESSION".to_string(), "ses3".to_string()),
        ];
        let resolve = |bin: &str| Some(format!("/opt/agents/{bin}"));
        let launch = h
            .launch(&LaunchInput {
                session: SessionIds {
                    stream: StreamId::new(1),
                    thread: ThreadId::new(2),
                    session: AgentSessionId::new(3),
                },
                workspace: &project,
                project_dir: &project,
                endpoints: &endpoints,
                identity_env: &identity,
                system_prompt,
                resume,
                text: &oxplow_plugin::core_text(),
                config,
                oxplow_executable: Path::new("/bin/oxplow"),
                home,
                resolve_program: &resolve,
            })
            .expect("it launches");
        Launched {
            launch,
            project,
            _dir: dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(capability: &str, id: &str, entry: &'static str) -> Implementation {
        Implementation {
            capability: capability.into(),
            id: id.into(),
            title: id.into(),
            extension: Some("oxplow-foundation".into()),
            source: Source::BuiltIn(entry),
            features: serde_json::json!({}),
            fields: serde_json::json!([]),
            id_pattern: None,
            config: serde_json::json!({}),
        }
    }

    /// What's declared is registered under its key; what no longer is goes.
    #[test]
    fn the_declared_harnesses_are_registered() {
        let r = HarnessRegistry::new(Arc::new(|| "claude".into()));
        register_built_ins(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "acp", "oxplow:acp"),
                declared("work_items", "oxplow", "oxplow:tasks"),
            ],
        );
        assert_eq!(r.names(), ["acp", "claude"]);
        assert_eq!(r.default().unwrap().id(), "claude");
        register_built_ins(&r, &[declared("agent_harness", "acp", "oxplow:acp")]);
        assert_eq!(r.names(), ["acp"]);
    }
}
