//! Watches the project's `.oxplow/project.yaml` and hot-reloads the
//! in-memory config when it changes on disk.
//!
//! Without this, a config edit made out-of-band — most importantly the
//! agent running `/oxplow:configure`, which writes a `testing:` block and
//! report collectors to `.oxplow/project.yaml` — wouldn't take effect until the app process
//! restarted, because config is otherwise read once at `Services` boot.
//! The config commands (`oxplow.config.set`, `oxplow.agent.*`) swap the
//! in-memory config as they write; this watcher covers every *other* way
//! the file changes.
//!
//! It also watches the default extension sources folder
//! (`.oxplow/extensions/`): a source added or deleted there restates the
//! listing rows that follow a source (`ExtensionActivation::restate_sources`),
//! without an activation. Sources `extensionSources` points elsewhere are
//! not watched; they show at the next activation.
//!
//! Spawned once at boot; it hears the project through the primary stream's
//! watch.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_config::OXPLOW_CONFIG_FILE;
use oxplow_fs_watch::{Batch, FsWatcher, RecursiveMode, Scope, Subscription};
use tracing::{debug, warn};

use crate::Services;

/// Holds the watch of what is outside the project. Drop to stop watching.
pub struct ConfigWatcher {
    _global: tokio::task::JoinHandle<()>,
}

impl ConfigWatcher {
    /// Follow the config. What is in the project comes through the primary
    /// stream's watch. Global settings follow their exact directory without
    /// recursion; only the metric library has a recursive watch. Missing
    /// directories are discovered by bounded polling of those known paths.
    pub fn spawn(services: Arc<Services>) -> Option<Self> {
        let layout = &services.layout;
        let sources_dir = layout
            .project_dir
            .join(oxplow_config::EXTENSION_SOURCES_DIR);
        let state_dir = layout
            .state_dir
            .strip_prefix(&layout.project_dir)
            .unwrap_or(Path::new(".oxplow"))
            .to_path_buf();
        let changes = services.stream_watches.subscribe_primary(
            Scope::default()
                .file(state_dir.join(OXPLOW_CONFIG_FILE))
                .file(state_dir.join("local.yaml"))
                .dir(oxplow_config::EXTENSION_SOURCES_DIR),
        );
        let global_dir = services.global_config_dir.clone();
        let project_dir = layout.state_dir.clone();
        let follow = move |mut changes: Subscription, services: Arc<Services>| {
            let sources_dir = sources_dir.clone();
            let project_dir = project_dir.clone();
            tokio::spawn(async move {
                // A save can fire create+modify; we only need to settle on the
                // final content.
                while let Some(batch) = changes.next_batch_after(Duration::from_millis(300)).await {
                    let says = what_changed(&batch, &sources_dir, &project_dir, None);
                    if says.sources {
                        if let Err(e) = services
                            .extension_activation
                            .restate_sources(services.clone())
                            .await
                        {
                            warn!(error = %e, "failed to restate the extension listing");
                        }
                    }
                    if says.config {
                        match services.reload_config_from_disk() {
                            Ok(()) => {
                                services.extension_activation.request();
                                debug!("reloaded settings after on-disk change");
                            }
                            Err(e) => warn!(error = %e, "failed to reload settings"),
                        }
                    }
                    if says.global_metrics {
                        services.global_metrics_changed();
                    }
                }
            });
        };
        follow(changes, services.clone());
        let task = tokio::spawn(follow_global(services, global_dir));
        Some(Self { _global: task })
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self._global.abort();
    }
}

/// Only existing, explicitly configured folders are watched. Missing folders
/// are discovered by polling their exact paths, never by watching an ancestor.
fn global_watch_paths(dir: &Path) -> Vec<(std::path::PathBuf, RecursiveMode)> {
    let mut paths = Vec::new();
    if dir.is_dir() {
        paths.push((dir.to_path_buf(), RecursiveMode::NonRecursive));
    }
    let metrics = dir.join("metrics");
    if metrics.is_dir() {
        paths.push((metrics, RecursiveMode::Recursive));
    }
    paths
}

fn config_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

async fn follow_global(services: Arc<Services>, dir: std::path::PathBuf) {
    let config = dir.join("global.yaml");
    let metrics = dir.join("metrics");
    // Re-read once when the task first polls, including a file created
    // between spawn and the first OS registration.
    let mut stamp = None;
    let mut paths: Vec<(std::path::PathBuf, RecursiveMode)> = Vec::new();
    let mut watcher = None;
    let mut subscription: Option<Subscription> = None;
    let mut poll = tokio::time::interval(Duration::from_millis(300));
    loop {
        poll.tick().await;
        let batch = subscription.as_mut().and_then(Subscription::try_batch);
        let wanted = global_watch_paths(&dir);
        let metrics_created = !paths.iter().any(|(path, _)| path == &metrics)
            && wanted.iter().any(|(path, _)| path == &metrics);
        // A folder replaced between polls needs a fresh OS registration too.
        let replaced = batch.as_ref().is_some_and(|batch| {
            batch
                .paths
                .iter()
                .any(|(path, _)| path == &dir || path == &metrics)
        });
        if wanted != paths || replaced || (watcher.is_none() && !wanted.is_empty()) {
            watcher = if wanted.is_empty() {
                None
            } else {
                FsWatcher::watch_paths(wanted.clone())
                    .inspect_err(|e| warn!(error = %e, "global settings watcher failed to start"))
                    .ok()
            };
            subscription = watcher.as_ref().map(FsWatcher::subscribe);
            paths = wanted;
        }
        let next_stamp = config_stamp(&config);
        let says = batch
            .as_ref()
            .map(|batch| {
                what_changed(
                    batch,
                    &services
                        .layout
                        .project_dir
                        .join(oxplow_config::EXTENSION_SOURCES_DIR),
                    &services.layout.state_dir,
                    Some(&dir),
                )
            })
            .unwrap_or_default();
        if says.config || next_stamp != stamp {
            match services.reload_config_from_disk() {
                Ok(()) => services.extension_activation.request(),
                Err(e) => warn!(error = %e, "failed to reload global settings"),
            }
        }
        stamp = next_stamp;
        if says.global_metrics || metrics_created {
            services.global_metrics_changed();
        }
    }
}

/// What a batch asks for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Says {
    /// Restate the listing rows that follow an extension source.
    sources: bool,
    /// Read the config again (it announces only what changed).
    config: bool,
    /// A global metric changed: announce it, as the metrics runner reads
    /// the global scope again on `ConfigChanged`.
    global_metrics: bool,
}

/// Changes it lost could be any of them, so a rescan asks for all of them
/// (the global metrics only when it covers their folder).
fn what_changed(
    batch: &Batch,
    sources_dir: &Path,
    project_state: &Path,
    global: Option<&Path>,
) -> Says {
    let global_metrics = global.map(|dir| dir.join("metrics"));
    let config_paths = [
        project_state.join(OXPLOW_CONFIG_FILE),
        project_state.join("local.yaml"),
    ]
    .into_iter()
    .chain(global.map(|dir| dir.join("global.yaml")))
    .collect::<Vec<_>>();
    let touches = |path: &Path| {
        batch
            .rescans
            .iter()
            .any(|root| path.starts_with(root) || root.starts_with(path))
    };
    let mut says = Says {
        sources: touches(sources_dir),
        config: config_paths.iter().any(|path| touches(path)),
        global_metrics: global_metrics.as_deref().is_some_and(|dir| {
            batch
                .rescans
                .iter()
                .any(|r| r.starts_with(dir) || dir.starts_with(r))
        }),
    };
    for (path, _) in &batch.paths {
        if path.starts_with(sources_dir) {
            says.sources = true;
            continue;
        }
        let is_config = config_paths.iter().any(|config| config == path);
        // A `.yaml`/`.yml` change under the global metrics dir is announced:
        // the runner re-reads the global scope on `ConfigChanged`.
        let is_global_metric = global_metrics.as_deref().is_some_and(|dir| {
            path.starts_with(dir)
                && matches!(
                    path.extension().and_then(|x| x.to_str()),
                    Some("yaml") | Some("yml")
                )
        });
        says.config |= is_config;
        says.global_metrics |= is_global_metric;
    }
    says
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_fs_watch::WatchEventKind;
    use std::path::PathBuf;

    fn says(paths: &[&str], rescans: &[&str]) -> Says {
        let batch = Batch {
            paths: paths
                .iter()
                .map(|p| (PathBuf::from(p), WatchEventKind::Modified))
                .collect(),
            rescans: rescans.iter().map(PathBuf::from).collect(),
        };
        what_changed(
            &batch,
            Path::new("/p/.oxplow/extensions"),
            Path::new("/p/.oxplow"),
            Some(Path::new("/g")),
        )
    }

    #[test]
    fn a_batch_asks_for_what_its_paths_touch() {
        assert_eq!(says(&["/p/.oxplow/local.sqlite"], &[]), Says::default());
        assert!(says(&["/p/.oxplow/project.yaml"], &[]).config);
        assert!(says(&["/p/.oxplow/local.yaml"], &[]).config);
        assert!(says(&["/g/global.yaml"], &[]).config);
        assert!(says(&["/p/.oxplow/local.yaml"], &[]).config);
        assert!(!says(&["/global/global.yaml"], &[]).config);
        assert!(!says(&["/p/.oxplow/unrelated.yaml"], &[]).config);
        // A global metric isn't project.yaml: reloading it would read the
        // same config and announce nothing, so it is its own ask.
        let metric = says(&["/g/metrics/a.yaml"], &[]);
        assert!(metric.global_metrics && !metric.config);
        assert!(!says(&["/g/metrics/a.txt"], &[]).global_metrics);
        let sources = says(&["/p/.oxplow/extensions/x/page.yaml"], &[]);
        assert!(sources.sources && !sources.config);
    }

    /// The lost change could be any of them.
    #[test]
    fn lost_changes_ask_for_everything() {
        let lost = says(&[], &["/p/.oxplow"]);
        assert!(lost.sources && lost.config && !lost.global_metrics);
        assert!(says(&[], &["/g/metrics"]).global_metrics);
        assert!(says(&[], &["/g"]).global_metrics);
    }

    #[test]
    fn unrelated_settings_and_rescans_do_not_reload_this_project() {
        for path in [
            "/g/project.yaml",
            "/g/local.yaml",
            "/other/project.yaml",
            "/p/nested/local.yaml",
        ] {
            assert_eq!(says(&[path], &[]), Says::default(), "{path}");
        }
        assert_eq!(says(&[], &["/other"]), Says::default());
    }

    #[test]
    fn global_watches_never_expand_to_an_existing_ancestor() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("missing/deep/config");
        assert!(global_watch_paths(&global).is_empty());
        std::fs::create_dir_all(&global).unwrap();
        assert_eq!(
            global_watch_paths(&global),
            vec![(global.clone(), RecursiveMode::NonRecursive)]
        );
        std::fs::create_dir(global.join("metrics")).unwrap();
        assert_eq!(
            global_watch_paths(&global),
            vec![
                (global.clone(), RecursiveMode::NonRecursive),
                (global.join("metrics"), RecursiveMode::Recursive),
            ]
        );
    }

    #[tokio::test]
    async fn a_global_directory_created_after_start_is_discovered_without_parent_watches() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let global = fx.svc.global_config_dir.clone();
        if global.exists() {
            std::fs::remove_dir_all(&global).unwrap();
        }
        let _watcher = ConfigWatcher {
            _global: tokio::spawn(follow_global(fx.svc.clone(), global.clone())),
        };
        assert!(global_watch_paths(&global).is_empty());
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(
            global.join("global.yaml"),
            "oxplow.snapshots.retentionDays: 23\n",
        )
        .unwrap();
        let changed = crate::test_fixtures::until_some(Duration::from_secs(3), || async {
            (fx.svc.config.read().unwrap().snapshot_retention_days == 23).then_some(())
        })
        .await;
        assert!(changed.is_some(), "the new global file was not loaded");
    }
}
