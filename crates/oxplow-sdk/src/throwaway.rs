//! A throwaway oxplow (P7.C6): in memory, over a copy of a project's
//! extension sources and which of them are on (the one under test always
//! is), deployed and activated as boot does it — the same staged generation, registries and declared
//! entities (empty, as before any collector ran) a running oxplow has. What
//! `extension test` and MCP `oxplow_preview_extension` run everything on, and what
//! `check` asks for a command registry when no oxplow is running.
//!
//! It never touches the project's own data: its database is in memory, its
//! keychain is in memory, its global config is under its own folder, and
//! the environment its programs see is the one it is given. Model calls go
//! to a fixture model answering from the example's replies — never a
//! provider, never a paid call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_ai::config::{BuiltinRole, ProviderConfig, RoleBinding};
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
    /// A throwaway over `project`'s extensions with `testing` on too,
    /// activated. Its programs see `env`. With `builds`, a copied source
    /// with a `build:` is built and deployed like the rest; without (a
    /// check, which runs no program), it isn't deployed.
    pub(crate) async fn start(
        project: &Path,
        testing: &str,
        env: HostEnv,
        builds: bool,
    ) -> Result<Host, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().to_path_buf();
        let settings = oxplow_config::extension_settings(project);
        for name in oxplow_app::extensions::known_names(project, &settings) {
            let source = settings.source(project, &name);
            if source.is_dir() {
                copy_dir(&source, &root.join(EXTENSIONS_DIR).join(&name))
                    .map_err(|e| e.to_string())?;
            }
        }
        // Which of them are on, and nothing else of its config: its
        // provider instances would start its programs here.
        let mut on: Vec<String> = settings.enabled().into_iter().map(|(n, _)| n).collect();
        // Every source here is on unless the person turned it off, as in a
        // source load.
        for name in oxplow_app::extensions::known_names(project, &settings) {
            if settings.source(project, &name).is_dir()
                && !settings.personal.disable.contains(&name)
                && !on.contains(&name)
            {
                on.push(name);
            }
        }
        if !on.iter().any(|n| n == testing) {
            on.push(testing.to_string());
        }
        let config = root.join(".oxplow/project.yaml");
        std::fs::create_dir_all(config.parent().expect("a folder")).map_err(|e| e.to_string())?;
        let yaml = serde_yaml::to_string(&serde_json::json!({ "extensions": on }))
            .map_err(|e| e.to_string())?;
        std::fs::write(config, yaml).map_err(|e| e.to_string())?;
        // And the ones the person turned off stay off in the copy.
        let off: Vec<&String> = settings
            .personal
            .disable
            .iter()
            .filter(|n| *n != testing)
            .collect();
        if !off.is_empty() {
            let yaml = serde_yaml::to_string(&serde_json::json!({ "disableExtensions": off }))
                .map_err(|e| e.to_string())?;
            std::fs::write(root.join(".oxplow/personal.yaml"), yaml).map_err(|e| e.to_string())?;
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
        // Boot deploys nothing; the author's own test run deploys what it
        // copied, as they would, confirmed.
        let mut deploys = Vec::new();
        for name in &on {
            let copied = root.join(EXTENSIONS_DIR).join(name);
            let built = oxplow_app::extension_store::Inputs::of(&copied)
                .run
                .is_some();
            if copied.is_dir() && (builds || !built) {
                if let Err(e) =
                    oxplow_app::extension_deploy::deploy(&svc, name, &root, None, true).await
                {
                    deploys.push(format!("{name}: {e}"));
                }
            }
        }
        let problems = svc
            .extension_activation
            .problems()
            .into_iter()
            .flat_map(|(owner, why)| why.into_iter().map(move |w| format!("{owner}: {w}")))
            .chain(deploys)
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
        for role in BuiltinRole::ALL {
            self.svc
                .ai
                .set_role(
                    &role.into(),
                    Some(RoleBinding::new(FIXTURE_INSTANCE, FIXTURE_INSTANCE)),
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Copy an extension's source; a folder its `build:` excludes
/// (`node_modules`) is linked instead, so a build in the copy still finds
/// it.
pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let exclude = oxplow_app::extension_store::Inputs::of(from).exclude;
    copy_tree(from, to, &exclude, "")
}

fn copy_tree(from: &Path, to: &Path, exclude: &[String], prefix: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let rel = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if exclude.iter().any(|d| d.trim_end_matches('/') == rel) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(entry.path(), &target)?;
            continue;
        }
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, exclude, &format!("{rel}/"))?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
