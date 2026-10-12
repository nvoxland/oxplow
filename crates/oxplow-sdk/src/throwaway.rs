//! A throwaway oxplow (P7.C6): in memory, over a copy of a project's
//! `oxplow/extensions/` and which of them it turned off, activated as boot
//! activates — the same staged generation, registries and declared
//! entities (empty, as before any collector ran) a running oxplow has. What
//! `extension test` and MCP `preview_extension` run everything on, and what
//! `check` asks for a command registry when no oxplow is running.
//!
//! It never touches the project's own data: its database is in memory, its
//! keychain is in memory, its global config is under its own folder, and
//! the environment its programs see is the one it is given. Model calls go
//! to a fixture model answering from the example's replies — never a
//! provider, never a paid call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_app::extensions::EXTENSIONS_DIR;
use oxplow_app::providers::host::HostEnv;

/// The fixture model's provider instance, which every role is bound to.
const FIXTURE_INSTANCE: &str = "fixture";

pub(crate) struct Host {
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) svc: Arc<oxplow_app::Services>,
    /// What its activation held back, and why (`<extension>: <reason>`).
    pub(crate) problems: Vec<String>,
}

impl Host {
    /// A throwaway over `project`'s extensions, activated. Its programs
    /// see `env`.
    pub(crate) async fn start(project: &Path, env: HostEnv) -> Result<Host, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().to_path_buf();
        let extensions = project.join(EXTENSIONS_DIR);
        if extensions.is_dir() {
            copy_dir(&extensions, &root.join(EXTENSIONS_DIR)).map_err(|e| e.to_string())?;
        }
        // Which of them the project turned off, and nothing else of its
        // config: its provider instances would start its programs here.
        let disabled = oxplow_config::disabled_extensions(project);
        if !disabled.is_empty() {
            let config = root.join(".oxplow/project.yaml");
            std::fs::create_dir_all(config.parent().expect("a folder"))
                .map_err(|e| e.to_string())?;
            let yaml = serde_yaml::to_string(&serde_json::json!({
                "extensions": { "disabled": disabled }
            }))
            .map_err(|e| e.to_string())?;
            std::fs::write(config, yaml).map_err(|e| e.to_string())?;
        }
        oxplow_app::vcs::GitProvider
            .init_repository(&root)
            .await
            .map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::in_memory_on_machine(
            &root,
            root.join(".oxplow/global-config"),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            env,
        )
        .await
        .map_err(|e| e.to_string())?;
        let problems = svc
            .extension_activation
            .problems()
            .into_iter()
            .flat_map(|(owner, why)| why.into_iter().map(move |w| format!("{owner}: {w}")))
            .collect();
        Ok(Host {
            _dir: dir,
            root,
            svc,
            problems,
        })
    }

    /// What running an extension's examples needs beyond activation, as
    /// boot starts it: effects and collectors hear the log (each example
    /// delivers its event with `settle`), and the job scheduler runs.
    pub(crate) fn start_runtime(&self) -> Result<(), String> {
        oxplow_app::effect_triggers::register(&self.svc);
        oxplow_app::collector_triggers::register(&self.svc);
        self.svc.jobs.spawn(
            self.svc.extension_activation.clone(),
            self.svc.commands.clone(),
        );
        self.answer_with(Vec::new())
    }

    /// Model calls from here on answer with `replies` in turn (the last
    /// repeats; none: an empty answer), every role bound to the fixture
    /// model.
    pub(crate) fn answer_with(
        &self,
        replies: Vec<oxplow_ai::fixture::FixtureReply>,
    ) -> Result<(), String> {
        self.svc
            .ai
            .providers()
            .register_host(Arc::new(oxplow_ai::fixture::FixtureModel::new(replies)));
        self.svc
            .ai
            .save_provider(
                ProviderConfig {
                    id: FIXTURE_INSTANCE.into(),
                    kind: oxplow_ai::fixture::KIND.into(),
                    base_url: None,
                },
                None,
            )
            .map_err(|e| e.to_string())?;
        for role in Role::ALL {
            self.svc
                .ai
                .set_role(
                    role,
                    Some(RoleBinding {
                        provider: FIXTURE_INSTANCE.into(),
                        model: FIXTURE_INSTANCE.into(),
                    }),
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
