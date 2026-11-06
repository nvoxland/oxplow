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
//! Spawned once at boot from the host binary; the returned handle holds
//! the `FsWatcher` for the process lifetime (drop cancels the watch).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_config::{OXPLOW_CONFIG_FILE, PERSONAL_CONFIG_FILE};
use oxplow_fs_watch::{Batch, FsWatcher, RecursiveMode};
use tracing::{debug, warn};

use crate::Services;

/// Holds the config-file watcher alive. Drop to stop watching.
pub struct ConfigWatcher {
    _fs: FsWatcher,
}

impl ConfigWatcher {
    /// Start watching `<project>/.oxplow/project.yaml`. Returns `None` if
    /// the watcher can't be created (logged, non-fatal — the app just won't
    /// hot-reload config).
    pub fn spawn(services: Arc<Services>) -> Option<Self> {
        let state_dir = services.layout.state_dir.clone();
        // Watch the `.oxplow` state dir non-recursively rather than the
        // file directly: editors (and atomic writers) replace the file via
        // rename, which breaks a single-file watch but is still visible as
        // a directory event. (The dir also holds the DB, so events are
        // filtered by file name below.)
        let mut watch = vec![(state_dir.clone(), RecursiveMode::NonRecursive)];
        // Also watch the user-global metric library
        // (`<global_config_dir>/metrics/`) so an edit there hot-reloads the
        // three-scope catalog (epic tsk213, P3). Only when it already exists —
        // watching a missing dir errors on some platforms.
        let sources_dir = services
            .layout
            .project_dir
            .join(oxplow_config::EXTENSION_SOURCES_DIR);
        // A folder to watch from the first write: a missing one errors on
        // some platforms, and an empty folder is not committed.
        if std::fs::create_dir_all(&sources_dir).is_ok() {
            watch.push((sources_dir.clone(), RecursiveMode::Recursive));
        }
        let global_metrics_dir = oxplow_config::global_config_dir().map(|d| d.join("metrics"));
        if let Some(dir) = global_metrics_dir.as_ref().filter(|d| d.is_dir()) {
            watch.push((dir.clone(), RecursiveMode::Recursive));
        }
        let watcher = match FsWatcher::watch_paths(watch) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, ?state_dir, "config watcher failed to start");
                return None;
            }
        };
        // A save can fire create+modify; we only need to settle on the final
        // content.
        let mut changes = watcher.subscribe();
        tokio::spawn(async move {
            while let Some(batch) = changes.next_batch_after(Duration::from_millis(300)).await {
                let says = what_changed(&batch, &sources_dir, global_metrics_dir.as_deref());
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
                        Ok(()) => debug!("reloaded project.yaml after on-disk change"),
                        Err(e) => warn!(error = %e, "failed to reload project.yaml"),
                    }
                }
            }
        });
        Some(Self { _fs: watcher })
    }
}

/// What a batch asks for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Says {
    /// Restate the listing rows that follow an extension source.
    sources: bool,
    /// Read the config again.
    config: bool,
}

/// Changes it lost could be any of them, so a rescan asks for both.
fn what_changed(batch: &Batch, sources_dir: &Path, global_metrics: Option<&Path>) -> Says {
    let mut says = Says {
        sources: !batch.rescans.is_empty(),
        config: !batch.rescans.is_empty(),
    };
    for (path, _) in &batch.paths {
        if path.starts_with(sources_dir) {
            says.sources = true;
            continue;
        }
        let is_config = path
            .file_name()
            .is_some_and(|n| n == OXPLOW_CONFIG_FILE || n == PERSONAL_CONFIG_FILE);
        // A `.yaml`/`.yml` change under the global metrics dir also triggers a
        // reload — the runner re-reads the global scope on the resulting
        // `ConfigChanged`.
        let is_global_metric = global_metrics.is_some_and(|dir| {
            path.starts_with(dir)
                && matches!(
                    path.extension().and_then(|x| x.to_str()),
                    Some("yaml") | Some("yml")
                )
        });
        says.config |= is_config || is_global_metric;
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
            Some(Path::new("/g/metrics")),
        )
    }

    #[test]
    fn a_batch_asks_for_what_its_paths_touch() {
        assert_eq!(says(&["/p/.oxplow/local.sqlite"], &[]), Says::default());
        assert!(says(&["/p/.oxplow/project.yaml"], &[]).config);
        assert!(says(&["/g/metrics/a.yaml"], &[]).config);
        assert!(!says(&["/g/metrics/a.txt"], &[]).config);
        let sources = says(&["/p/.oxplow/extensions/x/page.yaml"], &[]);
        assert!(sources.sources && !sources.config);
    }

    /// The lost change could be any of them.
    #[test]
    fn lost_changes_ask_for_everything() {
        let lost = says(&[], &["/p/.oxplow"]);
        assert!(lost.sources && lost.config);
    }
}
