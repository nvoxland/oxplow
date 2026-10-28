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
    /// The project checkout it was made from, where sources are found.
    pub(crate) checkout: PathBuf,
    pub(crate) svc: Arc<oxplow_app::Services>,
    /// What its activation held back, and why (`<extension>: <reason>`).
    pub(crate) problems: Vec<String>,
    /// How it was made, for [`Host::fork`]: the extensions on, where each
    /// copied source is (relative to `root`), its programs' environment,
    /// and whether built sources were deployed.
    on: Vec<String>,
    copied: std::collections::BTreeMap<String, PathBuf>,
    pub(crate) env: HostEnv,
    pub(crate) builds: bool,
    /// Its database as the deploys left it, before anything ran: what a
    /// fork starts from.
    deployed: oxplow_db::Database,
    /// A step changed something its materialized datasets haven't caught up
    /// with yet.
    stale: std::sync::atomic::AtomicBool,
}

impl Host {
    /// A throwaway over the extensions in `checkout` (the project's main
    /// checkout, or a worktree holding drafts) with `testing` on too,
    /// activated. Which are on: the required foundation, `testing` and what
    /// each of those requires or imports — never the default extensions, the
    /// project's list or one person's `enableExtensions` /
    /// `disableExtensions`, so a test runs the same for everyone, and oxplow's
    /// own pages run as they do with nothing added to them; where their sources are is the
    /// project's settings (its main checkout's), read in `checkout`. Its
    /// programs see `env`, and `extra` are more extensions to turn on (a
    /// fixture's `requires`). With `builds`, a copied source with a `build:` is
    /// built and deployed like the rest; without (a check, which runs no
    /// program), it isn't deployed.
    pub(crate) async fn start(
        checkout: &Path,
        testing: &str,
        extra: &[String],
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
        // Which are on: the required foundation, the one tested, and what
        // each of those `requires:` or imports from, in turn. Not the default
        // extensions: oxplow's own pages run complete without them, and an
        // extension that reads bundled's datasets says `requires:`. Never the project's list or
        // one person's: a test passes or fails on what the extension
        // declares, the same for everyone, and a sibling it doesn't name —
        // broken or not — isn't in it.
        // In the order they deploy: what one requires before it.
        fn visit(
            name: &str,
            checkout: &Path,
            settings: &oxplow_config::ExtensionSettings,
            on: &mut Vec<String>,
            seen: &mut std::collections::BTreeSet<String>,
        ) {
            if !seen.insert(name.to_string()) {
                return;
            }
            let source = settings.source(checkout, name);
            // One with no source here and none shipped isn't here at all:
            // what needs it says so.
            if !source.is_dir() {
                if oxplow_app::bundled_extensions::find(name).is_some() {
                    on.push(name.to_string());
                }
                return;
            }
            let ext = oxplow_app::extensions::load_extension_at(&source, name);
            let needs = ext.requires.iter().cloned().chain(
                ext.public_contracts
                    .imports
                    .iter()
                    .map(|i| i.contribution.extension.clone()),
            );
            for need in needs.collect::<Vec<_>>() {
                visit(&need, checkout, settings, on, seen);
            }
            on.push(name.to_string());
        }
        let (mut on, mut seen) = (Vec::new(), std::collections::BTreeSet::new());
        let roots = oxplow_app::bundled_extensions::BUNDLED
            .iter()
            .filter(|b| b.required)
            .map(|b| b.name)
            .chain([testing])
            .chain(extra.iter().map(String::as_str));
        for root in roots {
            visit(root, checkout, &settings, &mut on, &mut seen);
        }
        // Each source goes where the checkout keeps it, so a finding names
        // the file the author edits; one kept outside the checkout goes
        // under `.oxplow/extensions/`.
        let mut sources = std::collections::BTreeMap::new();
        let mut copied = std::collections::BTreeMap::new();
        for name in &on {
            let source = settings.source(checkout, name);
            if !source.is_dir() {
                continue;
            }
            let default = Path::new(EXTENSIONS_DIR).join(name);
            let rel = source
                .strip_prefix(checkout)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| default.clone());
            if rel != default {
                sources.insert(name.clone(), rel.to_string_lossy().into_owned());
            }
            copy_dir(&source, &root.join(&rel)).map_err(|e| e.to_string())?;
            copied.insert(name.clone(), rel);
        }
        let config = root.join(".oxplow/project.yaml");
        std::fs::create_dir_all(config.parent().expect("a folder")).map_err(|e| e.to_string())?;
        let yaml = serde_yaml::to_string(
            &serde_json::json!({ "extensions": on, "extensionSources": sources }),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(config, yaml).map_err(|e| e.to_string())?;
        Self::boot(dir, root, checkout.to_path_buf(), on, copied, env, builds).await
    }

    /// Whether `name` is an extension a throwaway here can turn on: shipped
    /// with oxplow, or a source in the checkout.
    pub(crate) fn knows(checkout: &Path, name: &str) -> bool {
        oxplow_app::bundled_extensions::find(name).is_some()
            || oxplow_config::extension_settings(checkout)
                .source(checkout, name)
                .is_dir()
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
        // Off the runtime: forks side by side wait their turn for the
        // snapshot's one connection.
        let deployed = self.deployed.clone();
        let db = tokio::task::spawn_blocking(move || deployed.copy_in_memory())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let svc = oxplow_app::Services::on_machine_with_database(
            &root,
            root.join(".oxplow/global-config"),
            Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            self.env.clone(),
            db,
        )
        .await
        .map_err(|e| e.to_string())?;
        // The copy's streams live in the copy's folder, not the one it was
        // copied from: a stage or a commit runs where its files are.
        {
            use oxplow_domain::stores::StreamStore as _;
            for mut stream in svc.stream_store.list().await.map_err(|e| e.to_string())? {
                if Path::new(&stream.worktree_path) == self.root {
                    stream.worktree_path = root.to_string_lossy().into_owned();
                    svc.stream_store
                        .upsert(&stream)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        let host = Host {
            _dir: dir,
            root,
            checkout: self.checkout.clone(),
            problems: problems(&svc, Vec::new()),
            svc,
            on: self.on.clone(),
            copied: self.copied.clone(),
            env: self.env.clone(),
            builds: self.builds,
            deployed: self.deployed.clone(),
            stale: Default::default(),
        };
        host.start_runtime()?;
        Ok(host)
    }

    /// Open oxplow over `root` (sources copied, settings written) and
    /// deploy what's on, as the author's own run would.
    async fn boot(
        dir: tempfile::TempDir,
        root: PathBuf,
        checkout: PathBuf,
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
            checkout,
            problems: problems(&svc, deploys),
            svc,
            deployed,
            on,
            copied,
            env,
            builds,
            stale: Default::default(),
        })
    }

    /// What running an extension's examples needs beyond activation, as
    /// boot starts it: effects and collectors hear the log (each example
    /// delivers its event with `settle`), and the job scheduler runs.
    pub(crate) fn start_runtime(&self) -> Result<(), String> {
        oxplow_app::effect_triggers::register(&self.svc);
        oxplow_app::collector_triggers::register(&self.svc);
        // A config change reaches what runs, as at boot; what it asks to
        // publish, [`Host::settle`] publishes.
        oxplow_app::config_reactors::register(&self.svc);
        // The one change loop, as at boot: materialized datasets refill.
        oxplow_app::datasets_changed::spawn(
            self.svc.db.clone(),
            self.svc.dataset_watermarks.clone(),
            self.svc.events.clone(),
            self.svc.assets.clone(),
            self.svc.event_pump.clone(),
            self.svc.page_runs.clone(),
        );
        self.svc.jobs.spawn(
            self.svc.extension_activation.clone(),
            self.svc.commands.clone(),
        );
        self.answer_with(Vec::new())
    }

    /// Let what the last step set off finish, as a person would wait for
    /// it: every reactor hears its events, and a generation one asked for
    /// is published. The materialized datasets are refilled before the next
    /// read ([`Host::current`]).
    pub(crate) async fn settle(&self) {
        let wait = std::time::Duration::from_secs(10);
        self.svc.event_pump.settle_all(wait).await;
        if self
            .svc
            .extension_activation
            .serve_pending(self.svc.clone())
            .await
        {
            self.svc.event_pump.settle_all(wait).await;
        }
        self.stale.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Every materialized dataset current, when a step changed anything
    /// since the last read — each refilled whatever its policy (`every 1h`
    /// too: a test has no clock to wait on). Before anything reads datasets.
    pub(crate) async fn current(&self) {
        if self.stale.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let assets = &self.svc.assets;
            // One that didn't register reads empty: the fixture reading it
            // says so.
            let _ = assets.sync_datasets().await;
            assets.recompute_all().await;
        }
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

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
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

    /// A test runs an extension with what it declares and what a fresh
    /// oxplow has, whoever runs it: its `requires:` (and theirs) are on, a
    /// sibling source it doesn't name is not — so a broken one is no noise
    /// in its report — and the person's own `disableExtensions` (here in
    /// the main checkout, as a worktree has no `personal.yaml`) turns
    /// nothing off.
    #[tokio::test]
    async fn a_test_runs_an_extension_with_what_it_declares_and_nothing_else() {
        let main = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(main.path())
            .await
            .unwrap();
        write(
            main.path(),
            ".oxplow/personal.yaml",
            "disableExtensions: [dep]\n",
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
            "manifest: 2\nname: acme\nintent:\n  purpose: test\nrequires: [dep]\n",
        );
        write(
            &wt,
            ".oxplow/extensions/dep/extension.yaml",
            "manifest: 2\nname: dep\nintent:\n  purpose: test\nrequires: [deeper]\n",
        );
        write(
            &wt,
            ".oxplow/extensions/deeper/extension.yaml",
            "manifest: 2\nname: deeper\nintent:\n  purpose: test\n",
        );
        write(
            &wt,
            ".oxplow/extensions/other/extension.yaml",
            "not: [valid\n",
        );

        let host = Host::start(&wt, "acme", &[], Arc::new(|_: &str| None), false)
            .await
            .unwrap();
        for name in ["acme", "dep", "deeper"] {
            assert!(host.on.iter().any(|n| n == name), "{name}: {:?}", host.on);
        }
        assert!(!host.on.iter().any(|n| n == "other"), "{:?}", host.on);
        assert!(
            !host.on.iter().any(|n| n == "oxplow-bundled"),
            "{:?}",
            host.on
        );
        assert!(!host.root.join(EXTENSIONS_DIR).join("other").exists());
        assert!(host.problems.is_empty(), "{:?}", host.problems);
        assert!(!host.root.join(".oxplow/personal.yaml").exists());
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

        let host = Host::start(project.path(), "acme", &[], Arc::new(|_: &str| None), false)
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
