//! A throwaway oxplow (P7.C6): in memory, over a copy of a project's
//! extension sources and which of them are on (the one under test always
//! is), deployed and activated as boot does it — the same staged generation, registries and declared
//! entities (empty, as before any collector ran) a running oxplow has. What
//! `extension test` runs everything on, and what
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
    /// How it was made, for [`Host::fork`]: the extensions on, where each
    /// copied source is (relative to `root`), its programs' environment,
    /// and whether built sources were deployed.
    on: Vec<String>,
    copied: std::collections::BTreeMap<String, PathBuf>,
    env: HostEnv,
    builds: bool,
    /// Its database as the deploys left it, before anything ran: what a
    /// fork starts from.
    deployed: oxplow_db::Database,
}

impl Host {
    /// A throwaway over the extensions in `checkout` (the project's main
    /// checkout, or a worktree holding drafts) with `testing` on too,
    /// activated. Which are on is the project's list and every source here —
    /// never one person's `enableExtensions` / `disableExtensions`, so a
    /// test runs the same for everyone; where their sources are is the
    /// project's settings (its main checkout's), read in `checkout`. Its
    /// programs see `env`. With `builds`, a copied source with a `build:` is
    /// built and deployed like the rest; without (a check, which runs no
    /// program), it isn't deployed.
    pub(crate) async fn start(
        checkout: &Path,
        testing: &str,
        env: HostEnv,
        builds: bool,
    ) -> Result<Host, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().to_path_buf();
        let project = &oxplow_app::vcs::GitProvider
            .main_checkout(checkout)
            .await
            .unwrap_or_else(|| checkout.to_path_buf());
        let settings = oxplow_config::extension_settings(project);
        // Each source goes where the checkout keeps it, so a finding names
        // the file the author edits; one kept outside the checkout goes
        // under `.oxplow/extensions/`.
        let mut sources = std::collections::BTreeMap::new();
        let mut copied = std::collections::BTreeMap::new();
        for name in oxplow_app::extensions::known_names(checkout, &settings) {
            let source = settings.source(checkout, &name);
            if !source.is_dir() {
                continue;
            }
            let default = Path::new(EXTENSIONS_DIR).join(&name);
            let rel = source
                .strip_prefix(checkout)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| default.clone());
            if rel != default {
                sources.insert(name.clone(), rel.to_string_lossy().into_owned());
            }
            copy_dir(&source, &root.join(&rel)).map_err(|e| e.to_string())?;
            copied.insert(name, rel);
        }
        // Which of them are on — the project's, never one person's
        // `enableExtensions` / `disableExtensions`, so a test passes or
        // fails the same for everyone — and nothing else of its config: its
        // provider instances would start its programs here.
        let mut on: Vec<String> = settings.team.clone();
        // Every source here is on, as in a source load.
        for name in oxplow_app::extensions::known_names(checkout, &settings) {
            if settings.source(checkout, &name).is_dir() && !on.contains(&name) {
                on.push(name);
            }
        }
        if !on.iter().any(|n| n == testing) {
            on.push(testing.to_string());
        }
        let config = root.join(".oxplow/project.yaml");
        std::fs::create_dir_all(config.parent().expect("a folder")).map_err(|e| e.to_string())?;
        let yaml = serde_yaml::to_string(
            &serde_json::json!({ "extensions": on, "extensionSources": sources }),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(config, yaml).map_err(|e| e.to_string())?;
        Self::boot(dir, root, on, copied, env, builds).await
    }

    /// Another throwaway like this one as it was deployed, before anything
    /// ran in it: its files copied and its database copied, so it boots
    /// what was deployed — nothing is built or deployed again. What one
    /// example makes, the next one's fork doesn't see.
    pub(crate) async fn fork(&self) -> Result<Host, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().to_path_buf();
        copy_all(&self.root, &root).map_err(|e| e.to_string())?;
        oxplow_app::vcs::GitProvider
            .init_repository(&root)
            .await
            .map_err(|e| e.to_string())?;
        let db = self.deployed.copy_in_memory().map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::on_machine_with_database(
            &root,
            root.join(".oxplow/global-config"),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            self.env.clone(),
            db,
        )
        .await
        .map_err(|e| e.to_string())?;
        let host = Host {
            _dir: dir,
            root,
            problems: problems(&svc, Vec::new()),
            svc,
            on: self.on.clone(),
            copied: self.copied.clone(),
            env: self.env.clone(),
            builds: self.builds,
            deployed: self.deployed.clone(),
        };
        host.start_runtime()?;
        Ok(host)
    }

    /// Open oxplow over `root` (sources copied, settings written) and
    /// deploy what's on, as the author's own run would.
    async fn boot(
        dir: tempfile::TempDir,
        root: PathBuf,
        on: Vec<String>,
        copied: std::collections::BTreeMap<String, PathBuf>,
        env: HostEnv,
        builds: bool,
    ) -> Result<Host, String> {
        oxplow_app::vcs::GitProvider
            .init_repository(&root)
            .await
            .map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::in_memory_on_machine(
            &root,
            root.join(".oxplow/global-config"),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            env.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        // Boot deploys nothing; the author's own test run deploys what it
        // copied, as they would, confirmed.
        let mut deploys = Vec::new();
        for name in &on {
            let Some(source) = copied.get(name) else {
                continue;
            };
            let built = oxplow_app::extension_store::Inputs::of(&root.join(source))
                .run
                .is_some();
            if builds || !built {
                if let Err(e) = oxplow_app::extension_deploy::deploy(&svc, name, true).await {
                    deploys.push(format!("{name}: {e}"));
                }
            }
        }
        let db = svc.db.clone();
        let deployed = tokio::task::spawn_blocking(move || db.copy_in_memory())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Host {
            _dir: dir,
            root,
            problems: problems(&svc, deploys),
            svc,
            deployed,
            on,
            copied,
            env,
            builds,
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
/// What `svc`'s activation held back and the `deploys` that failed, as
/// `<extension>: <reason>`.
fn problems(svc: &oxplow_app::Services, deploys: Vec<String>) -> Vec<String> {
    svc.extension_activation
        .problems()
        .into_iter()
        .flat_map(|(owner, why)| why.into_iter().map(move |w| format!("{owner}: {w}")))
        .chain(deploys)
        .collect()
}

/// Copy throwaway `from` to `to` whole but its git repository: symlinks
/// stay symlinks, a deployment's read-only files stay read-only.
fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
        } else if kind.is_dir() {
            if from.join(".git") == entry.path() {
                continue;
            }
            copy_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A test runs with the project's extensions, whoever runs it: the
    /// person's own `disableExtensions` (here in the main checkout, as a
    /// worktree has no `personal.yaml`) doesn't turn one off in the
    /// throwaway, so a fixture passes or fails the same for everyone and in
    /// CI. A source that doesn't load is a warning beside the one tested.
    #[tokio::test]
    async fn a_test_runs_with_the_projects_extensions_not_the_persons() {
        let main = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(main.path())
            .await
            .unwrap();
        write(
            main.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [other]\n",
        );
        let parent = tempfile::tempdir().unwrap();
        let wt = parent.path().join("wt");
        let added = std::process::Command::new("git")
            .args(["worktree", "add", "-q", "-b", "draft"])
            .arg(&wt)
            .current_dir(main.path())
            .status()
            .unwrap();
        assert!(added.success());
        write(
            &wt,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        write(
            &wt,
            ".oxplow/extensions/other/extension.yaml",
            "not: [valid\n",
        );

        let host = Host::start(&wt, "acme", Arc::new(|_: &str| None), false)
            .await
            .unwrap();
        assert!(host.on.iter().any(|n| n == "other"), "{:?}", host.on);
        assert!(!host.root.join(".oxplow/personal.yaml").exists());
        assert!(host.root.join(EXTENSIONS_DIR).join("acme").is_dir());
    }

    /// An extension whose source the project keeps elsewhere
    /// (`extensionSources`, as oxplow's own are kept in `extensions/`) is
    /// copied to the same place in the throwaway, so every finding names the
    /// file the author edits.
    #[tokio::test]
    async fn a_source_kept_elsewhere_is_reported_where_it_is() {
        let project = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(project.path())
            .await
            .unwrap();
        write(
            project.path(),
            ".oxplow/project.yaml",
            "extensionSources: { acme: extensions/acme }\n",
        );
        write(
            project.path(),
            "extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n  examples:\n    - { name: unpaired, input: { page: none }, expect: nothing }\n",
        );

        let host = Host::start(project.path(), "acme", Arc::new(|_: &str| None), false)
            .await
            .unwrap();
        assert!(host.root.join("extensions/acme").is_dir());
        let report = crate::extension_test::test_extension(project.path(), "acme", false)
            .await
            .unwrap();
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.starts_with("extensions/acme/extension.yaml:")),
            "{:?}",
            report.warnings
        );
        assert!(
            !report.warnings.iter().any(|w| w.contains(EXTENSIONS_DIR)),
            "{:?}",
            report.warnings
        );
    }
}
