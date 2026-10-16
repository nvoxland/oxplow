//! The built-in agent harnesses (`agent_harness` implementations,
//! `.context/agent-model.md`): Claude Code, Codex and opencode,
//! each an `AgentHarness` that writes its own runtime under
//! `.oxplow/runtime/` and launches its sessions. Core registers the ones a
//! project's extensions declare, under the key each declaration gives.

mod claude;
mod codex;
mod opencode;
mod shared;
#[cfg(test)]
mod text_contract;

use std::sync::Arc;

/// The client of Codex's app-server daemon its sessions are spoken to
/// through.
pub use codex::app_server as codex_app_server;

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
        _ => return None,
    })
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

    use oxplow_domain::agent::harness::{
        AgentHarness, Endpoints, Launch, LaunchInput, RuntimeRoots, SessionIds,
    };
    use oxplow_domain::{AgentSessionId, StreamId, ThreadId};

    pub struct Launched {
        pub launch: Launch,
        pub project: PathBuf,
        /// The stream's worktree, apart from the project.
        pub workspace: PathBuf,
        _dir: tempfile::TempDir,
    }

    /// Where the stream's worktree is, under the project.
    const WORKSPACE: &str = "worktrees/s1";

    /// The roots of a project with no worktrees.
    pub fn project_only(project: &Path) -> RuntimeRoots {
        RuntimeRoots {
            project_dir: project.to_path_buf(),
            workspaces: Vec::new(),
        }
    }

    /// Run a harness's call to its end, for a test.
    pub fn block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
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

    /// What a test launch varies; the rest is fixed (session ses3 on
    /// thread thr2 of stream str1, a scratch project).
    #[derive(Default)]
    struct Run<'a> {
        resume: Option<&'a str>,
        fork: bool,
        system_prompt: Option<&'a str>,
        config: Option<&'a serde_json::Value>,
        home: Option<&'a Path>,
        /// Programs left off the search path.
        without: &'a [&'a str],
    }

    /// `h` launched for session ses3 (thread thr2, stream str1) in a scratch
    /// project, its programs resolved under `/opt/agents`.
    pub fn launch_in(
        h: &dyn AgentHarness,
        resume: Option<&str>,
        system_prompt: Option<&str>,
        config: &serde_json::Value,
    ) -> Launched {
        run(
            h,
            Run {
                resume,
                system_prompt,
                config: Some(config),
                ..Run::default()
            },
            tempfile::tempdir().unwrap(),
        )
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
            let workspace = dir.path().join(WORKSPACE).to_string_lossy().into_owned();
            setup(home.path(), &workspace);
            run(
                h,
                Run {
                    resume: Some(resume),
                    home: Some(home.path()),
                    ..Run::default()
                },
                dir,
            )
        };
        drop(home);
        launched
    }

    /// `h` launched as [`launch_in`], with the person's home at
    /// `<project>/home`, forking `resume` when `fork`.
    pub fn launch_at_home(
        h: &dyn AgentHarness,
        resume: Option<&str>,
        fork: bool,
        system_prompt: Option<&str>,
    ) -> Launched {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        run(
            h,
            Run {
                resume,
                fork,
                system_prompt,
                home: Some(&home),
                ..Run::default()
            },
            dir,
        )
    }

    /// `h` launched as [`launch_at_home`] with `bin` nowhere oxplow looks,
    /// for a launch that is refused.
    pub fn try_launch_at_home_without(
        h: &dyn AgentHarness,
        bin: &str,
    ) -> Result<Launched, oxplow_domain::agent::harness::HarnessError> {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        try_run(
            h,
            Run {
                home: Some(&home),
                without: &[bin],
                ..Run::default()
            },
            dir,
        )
    }

    /// `h` launched to fork saved conversation `resume`, as [`launch_in`].
    pub fn launch_fork(h: &dyn AgentHarness, resume: &str) -> Launched {
        run(
            h,
            Run {
                resume: Some(resume),
                fork: true,
                ..Run::default()
            },
            tempfile::tempdir().unwrap(),
        )
    }

    /// `h` launched with `config`, as [`launch_in`], for a launch that is
    /// refused.
    pub fn try_launch_in(
        h: &dyn AgentHarness,
        config: &serde_json::Value,
    ) -> Result<Launched, oxplow_domain::agent::harness::HarnessError> {
        try_run(
            h,
            Run {
                config: Some(config),
                ..Run::default()
            },
            tempfile::tempdir().unwrap(),
        )
    }

    fn run(h: &dyn AgentHarness, r: Run<'_>, dir: tempfile::TempDir) -> Launched {
        try_run(h, r, dir).expect("it launches")
    }

    fn try_run(
        h: &dyn AgentHarness,
        r: Run<'_>,
        dir: tempfile::TempDir,
    ) -> Result<Launched, oxplow_domain::agent::harness::HarnessError> {
        let project = dir.path().to_path_buf();
        let workspace = project.join(WORKSPACE);
        std::fs::create_dir_all(&workspace).unwrap();
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
        // Programs resolve under `<scratch>/opt/agents`: a search path
        // holding each one a launch asks for.
        let agents = project.join("opt/agents");
        std::fs::create_dir_all(&agents).unwrap();
        for bin in ["claude", "codex", "opencode"] {
            if !r.without.contains(&bin) {
                std::fs::write(agents.join(bin), "").unwrap();
            }
        }
        let launch = block(h.launch(&LaunchInput {
            session: SessionIds {
                stream: StreamId::new(1),
                thread: ThreadId::new(2),
                session: AgentSessionId::new(3),
            },
            workspace: workspace.clone(),
            project_dir: project.clone(),
            endpoints,
            identity_env: identity,
            system_prompt: r.system_prompt.map(str::to_string),
            resume: r.resume.map(str::to_string),
            fork: r.fork,
            text: oxplow_agent_text::core_text(),
            config: r.config.cloned().unwrap_or_else(|| serde_json::json!({})),
            home: r.home.map(Path::to_path_buf),
            search_path: vec![agents],
        }))?;
        Ok(Launched {
            launch,
            project,
            workspace,
            _dir: dir,
        })
    }
}
