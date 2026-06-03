//! fs watcher that keeps the `wiki_page` rows in sync with the
//! `.oxplow/wiki/` markdown files.
//!
//! Mirrors `src/git/notes-watch.ts` from main: an initial scan on
//! start, then debounced per-slug re-syncs on file change. Wraps
//! [`oxplow_fs_watch::FsWatcher`] for the debouncing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::{SqlitePageRefStore, SqliteWikiPageStore};
use oxplow_fs_watch::FsWatcher;
use tracing::{info, warn};

use crate::events::{EventBus, OxplowEvent};
use crate::file_ref_version;
use crate::snapshot_capture::SnapshotCaptureService;
use crate::wiki_pages;

/// Spawn a wiki-page watcher. Holding the returned struct keeps the
/// watcher alive; dropping it cancels the OS handles + the relay
/// task (channel close).
pub struct WikiPagesWatcher {
    _watcher: FsWatcher,
}

impl WikiPagesWatcher {
    /// Boot — runs the initial scan synchronously, then attaches the
    /// debounced fs watcher. Errors during scan are logged but don't
    /// prevent the watcher from starting.
    pub async fn spawn(
        project_dir: PathBuf,
        store: Arc<SqliteWikiPageStore>,
        page_refs: Arc<SqlitePageRefStore>,
        events: EventBus,
        snapshot_capture: Option<Arc<SnapshotCaptureService>>,
    ) -> Option<Self> {
        let dir = wiki_pages::wiki_pages_dir(&project_dir);
        std::fs::create_dir_all(&dir).ok();

        if let Err(err) =
            wiki_pages::scan_and_sync_all_with_refs(&project_dir, &store, Some(&page_refs)).await
        {
            warn!(?err, "wiki pages initial scan failed");
        } else {
            info!(dir = %dir.display(), "wiki pages initial scan complete");
        }

        let watcher = match FsWatcher::watch(&dir) {
            Ok(w) => w,
            Err(err) => {
                warn!(?err, "wiki pages watcher failed to start");
                return None;
            }
        };
        // Debounced: editors save `.md` files in a few rapid writes;
        // one re-sync per slug per burst is enough.
        let mut rx = watcher.subscribe_debounced(Duration::from_millis(250));

        let project_dir_for_loop = project_dir.clone();
        let store_for_loop = store.clone();
        let page_refs_for_loop = page_refs.clone();
        let events_for_loop = events.clone();
        let snapshot_capture_for_loop = snapshot_capture.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if evt.path.extension().and_then(|s| s.to_str()) != Some("md") {
                            continue;
                        }
                        let Some(slug) = evt.path.file_stem().and_then(|s| s.to_str()) else {
                            continue;
                        };
                        // Pin the snapshot the wiki -> file edges
                        // for this re-sync should be tagged against.
                        // Without a snapshot service (tests) the
                        // edges land without version data.
                        let file_version = match snapshot_capture_for_loop.as_ref() {
                            Some(svc) => {
                                match svc
                                    .store()
                                    .latest_snapshot_id_for_stream(oxplow_domain::StreamId::from(
                                        svc.stream_id().to_string(),
                                    ))
                                    .await
                                {
                                    Ok(Some(snapshot_id)) => file_ref_version::resolve(
                                        svc.store(),
                                        svc.project_dir(),
                                        snapshot_id,
                                    )
                                    .await
                                    .ok(),
                                    _ => None,
                                }
                            }
                            None => None,
                        };
                        if let Err(err) = wiki_pages::sync_from_disk_with_refs_versioned(
                            &project_dir_for_loop,
                            &store_for_loop,
                            Some(&page_refs_for_loop),
                            slug,
                            file_version,
                        )
                        .await
                        {
                            warn!(slug, ?err, "wiki page resync failed");
                            continue;
                        }
                        events_for_loop.emit(OxplowEvent::WikiPagesChanged {
                            slug: slug.to_string(),
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "wiki pages watcher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Some(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::events::EventBus;

    /// Touching `.oxplow/wiki/<slug>.md` makes the watcher emit
    /// `WikiPagesChanged { slug }` carrying exactly that file's stem,
    /// so subscribers can filter by their own slug.
    #[tokio::test]
    async fn watcher_emits_slug_on_file_change() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let wiki_dir = crate::wiki_pages::wiki_pages_dir(&project);
        std::fs::create_dir_all(&wiki_dir).unwrap();

        let db = oxplow_db::Database::in_memory();
        let store = Arc::new(oxplow_db::SqliteWikiPageStore::new(db.clone()));
        let page_refs = Arc::new(oxplow_db::SqlitePageRefStore::new(db));
        let events = EventBus::new();
        let mut rx = events.subscribe();

        let _watcher = WikiPagesWatcher::spawn(project.clone(), store, page_refs, events, None)
            .await
            .expect("watcher to spawn");

        // Give the OS-level watcher a moment to attach before we
        // poke the directory.
        tokio::time::sleep(Duration::from_millis(50)).await;

        std::fs::write(wiki_dir.join("hello-world.md"), "# Hello\nbody\n").unwrap();

        // 250ms debounce + scheduling slack.
        let evt = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Ok(OxplowEvent::WikiPagesChanged { slug }) => return slug,
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        panic!("event bus closed before WikiPagesChanged");
                    }
                }
            }
        })
        .await
        .expect("WikiPagesChanged event within 3s");

        assert_eq!(evt, "hello-world");
    }
}
