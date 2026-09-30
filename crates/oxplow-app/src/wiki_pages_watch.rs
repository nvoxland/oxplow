//! fs watcher that keeps the `wiki_page` rows in sync with the
//! `.oxplow/wiki/` markdown files: an initial scan on start, then
//! debounced per-slug re-syncs ([`wiki_pages::sync_page`], logged as
//! `system:wiki_watch`) on file change. A page `knowledge.write_page`
//! just wrote syncs to a no-op (its body hash matches). Wraps
//! [`oxplow_fs_watch::FsWatcher`] for the debouncing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::{Database, SqliteWikiPageStore};
use oxplow_domain::EventSchemaRegistry;
use oxplow_fs_watch::FsWatcher;
use tracing::{info, warn};

use crate::events::{EventBus, OxplowEvent};
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
        db: Database,
        schemas: Arc<EventSchemaRegistry>,
        store: Arc<SqliteWikiPageStore>,
        events: EventBus,
    ) -> Option<Self> {
        let dir = wiki_pages::wiki_pages_dir(&project_dir);
        std::fs::create_dir_all(&dir).ok();

        match wiki_pages::scan_and_sync_all(&db, &schemas, &project_dir, &store).await {
            Ok(report) if report.failures.is_empty() => {
                info!(dir = %dir.display(), synced = report.synced, "wiki pages initial scan complete");
            }
            Ok(report) => {
                let failed: Vec<&str> = report.failures.iter().map(|(s, _)| s.as_str()).collect();
                warn!(
                    synced = report.synced,
                    ?failed,
                    "wiki pages initial scan completed with per-page failures"
                );
            }
            Err(err) => warn!(?err, "wiki pages initial scan failed"),
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
                        match wiki_pages::sync_page(&db, &schemas, &project_dir, slug).await {
                            Ok(true) => events.emit(OxplowEvent::WikiPagesChanged {
                                slug: slug.to_string(),
                            }),
                            Ok(false) => {}
                            Err(err) => warn!(slug, ?err, "wiki page resync failed"),
                        }
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
        let schemas = Arc::new(EventSchemaRegistry::core());
        let events = EventBus::new();
        let mut rx = events.subscribe();

        let _watcher = WikiPagesWatcher::spawn(project.clone(), db, schemas, store, events)
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
