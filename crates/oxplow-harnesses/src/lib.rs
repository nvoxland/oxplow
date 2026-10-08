//! The built-in agent harnesses (`agent_harness` implementations,
//! `.context/agent-model.md`): Claude Code, Codex, opencode and ACP agents,
//! each an `AgentHarness` that writes its own runtime under
//! `.oxplow/runtime/` and launches its sessions. Core registers the ones a
//! project's extensions declare, under the key each declaration gives.

mod acp;
mod claude;
mod codex;
mod opencode;
mod shared;

use std::sync::Arc;

use oxplow_domain::agent::acp_adapter::AcpAdapter;
use oxplow_domain::agent::harness::AgentHarness;

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

/// The ACP adapter a built-in `entry` declares as `id`, from its `config`
/// (`{ command, args?, env?, systemPrompt?: meta|prompt }`); `None` for an
/// entry that isn't one.
pub fn acp_adapter(
    entry: &str,
    id: &str,
    title: &str,
    config: &serde_json::Value,
) -> Option<Result<AcpAdapter, String>> {
    (entry == "oxplow:acp-adapter").then(|| AcpAdapter::from_config(id, title, config))
}

/// A harness's key and title, as its declaration gives them.
struct Named {
    id: String,
    title: String,
}

/// Launching a harness in a scratch project, for its tests.
#[cfg(test)]
mod test_launch {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use oxplow_domain::agent::harness::{AgentHarness, Endpoints, Launch, LaunchInput, SessionIds};
    use oxplow_domain::{AgentSessionId, StreamId, ThreadId};

    pub struct Launched {
        pub launch: Launch,
        pub project: PathBuf,
        _dir: tempfile::TempDir,
    }

    /// A PTY launch's command and env. Its bearer (`secret-bearer`) is in
    /// the env only: the command's text any process can list.
    pub fn pty(launch: &Launch) -> (&str, std::collections::HashMap<&str, &str>) {
        match &launch.spec {
            oxplow_domain::agent::harness::LaunchSpec::Pty { command, env } => {
                assert!(
                    !command.contains("secret-bearer"),
                    "the bearer is in the command: {command}"
                );
                (
                    command,
                    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
                )
            }
            other => panic!("not a PTY launch: {other:?}"),
        }
    }

    /// Whether only its owner may read `path`.
    pub fn owner_only(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o077 == 0
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
            hook_token: "secret-bearer".into(),
        };
        let identity = vec![
            ("OXPLOW_HOOK_TOKEN".to_string(), "secret-bearer".to_string()),
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
                text: &oxplow_agent_text::core_text(),
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
