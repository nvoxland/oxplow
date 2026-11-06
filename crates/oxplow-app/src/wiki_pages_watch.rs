//! fs watcher that keeps the `wiki_page` rows in sync with the
//! `.oxplow/wiki/` markdown files: an initial scan on start, then
//! debounced per-slug re-syncs ([`wiki_pages::sync_page`], logged as
//! `system:wiki_watch`) on file change. A page `oxplow.knowledge.write_page`
//! just wrote syncs to a no-op (its body hash matches). Wraps
//! [`oxplow_fs_watch::FsWatcher`] for the debouncing.

use oxplow_domain::vocabulary::VocabularyHandle;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::{Database, SqliteWikiPageStore};
use oxplow_fs_watch::{Batch, FsWatcher};
use tracing::{info, warn};

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
    /// prevent the watcher from starting. While the wiki isn't the
    /// project's knowledge store (`wiki`) its files aren't the record's:
    /// the record left from before is emptied instead, and changes are
    /// ignored until it is again (`knowledge::KnowledgeSwitch` restates it).
    pub async fn spawn(
        project_dir: PathBuf,
        db: Database,
        vocabulary: VocabularyHandle,
        store: Arc<SqliteWikiPageStore>,
        wiki: Arc<crate::knowledge::WikiFollow>,
    ) -> Option<Self> {
        let dir = wiki_pages::wiki_pages_dir(&project_dir);
        std::fs::create_dir_all(&dir).ok();

        let scanned = if wiki.live() {
            wiki_pages::scan_and_sync_all(&db, &vocabulary, &project_dir, &store).await
        } else {
            crate::knowledge::clear_record(&db)
                .await
                .map(|()| wiki_pages::ScanReport::default())
        };
        match scanned {
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
        // Editors save `.md` files in a few rapid writes; one re-sync per
        // slug per burst is enough.
        let mut changes = watcher.subscribe();
        let sync = Sync {
            db,
            vocabulary,
            project_dir,
            store,
        };
        tokio::spawn(async move {
            while let Some(batch) = changes.next_batch_after(Duration::from_millis(250)).await {
                if wiki.live() {
                    sync.batch(batch).await;
                }
            }
        });

        Some(Self { _watcher: watcher })
    }
}

/// Restates `wiki_page` rows from the files.
struct Sync {
    db: Database,
    vocabulary: VocabularyHandle,
    project_dir: PathBuf,
    store: Arc<SqliteWikiPageStore>,
}

impl Sync {
    /// Each changed page's row. The row write is the announcement: the UI
    /// re-reads on its `datasetsChanged`. If changes were lost, every page.
    async fn batch(&self, batch: Batch) {
        if !batch.rescans.is_empty() {
            let scanned = wiki_pages::scan_and_sync_all(
                &self.db,
                &self.vocabulary,
                &self.project_dir,
                &self.store,
            )
            .await;
            if let Err(err) = scanned {
                warn!(?err, "wiki pages rescan failed");
            }
            return;
        }
        for (path, _) in batch.paths {
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Err(err) =
                wiki_pages::sync_page(&self.db, &self.vocabulary, &self.project_dir, slug).await
            {
                warn!(slug, ?err, "wiki page resync failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// A page's row follows its file, and when changes were lost every page's
    /// does, with no OS involved.
    #[tokio::test]
    async fn a_batch_restates_rows_and_a_rescan_restates_them_all() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let wiki_dir = crate::wiki_pages::wiki_pages_dir(&project);
        std::fs::create_dir_all(&wiki_dir).unwrap();
        let db = oxplow_db::Database::in_memory();
        let store = Arc::new(oxplow_db::SqliteWikiPageStore::new(db.clone()));
        let sync = Sync {
            db,
            vocabulary: VocabularyHandle::core(),
            project_dir: project,
            store: store.clone(),
        };
        std::fs::write(wiki_dir.join("one.md"), "# One\n").unwrap();
        std::fs::write(wiki_dir.join("two.md"), "# Two\n").unwrap();

        sync.batch(Batch {
            paths: vec![(
                wiki_dir.join("one.md"),
                oxplow_fs_watch::WatchEventKind::Modified,
            )],
            rescans: Vec::new(),
        })
        .await;
        assert!(store.body("one").await.unwrap().is_some());
        assert!(store.body("two").await.unwrap().is_none());

        sync.batch(Batch {
            paths: Vec::new(),
            rescans: vec![wiki_dir],
        })
        .await;
        assert!(store.body("two").await.unwrap().is_some());
    }

    /// Touching `.oxplow/wiki/<slug>.md` makes the watcher restate that
    /// page's row (title and body from the file) — the row write is the
    /// announcement the UI re-reads on; there is no wiki event of its own.
    #[tokio::test]
    async fn watcher_restates_the_row_on_file_change() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let wiki_dir = crate::wiki_pages::wiki_pages_dir(&project);
        std::fs::create_dir_all(&wiki_dir).unwrap();

        let db = oxplow_db::Database::in_memory();
        let store = Arc::new(oxplow_db::SqliteWikiPageStore::new(db.clone()));
        let vocabulary = VocabularyHandle::core();

        let wiki = Arc::new(crate::knowledge::WikiFollow::default());
        wiki.set_serves_for_test("oxplow");
        let _watcher =
            WikiPagesWatcher::spawn(project.clone(), db, vocabulary, store.clone(), wiki)
                .await
                .expect("watcher to spawn");

        // The OS-level watcher may still be attaching on a busy machine:
        // write the file again until its row appears, with a generous
        // deadline (the debounce is 250ms).
        let row = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                std::fs::write(wiki_dir.join("hello-world.md"), "# Hello\nbody\n").unwrap();
                for _ in 0..16 {
                    if let Some(row) = store.body("hello-world").await.unwrap() {
                        return row;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        })
        .await
        .expect("the page's row within 30s");

        assert_eq!(row, ("Hello".to_string(), "# Hello\nbody\n".to_string()));
    }
}
