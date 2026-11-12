//! Keeps oxwiki's record in step with its files: a scan whenever oxwiki
//! becomes an enabled store (at boot, or turned back on), then per-page
//! re-syncs ([`pages::sync_page`]) on file change, heard through the primary
//! stream's watch. A page `oxplow.knowledge.write_page` just wrote syncs to a
//! no-op (its body hash matches). While oxwiki is off its files are left
//! alone and its recorded pages stay, hidden.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::Database;
use oxplow_domain::knowledge::OXWIKI_DIR;
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_fs_watch::{Batch, Scope, Subscription};
use tracing::{info, warn};

use crate::pages;
use crate::OxwikiPresence;

/// What of a worktree this follows.
pub fn scope() -> Scope {
    Scope::default().dir(OXWIKI_DIR)
}

/// Restates oxwiki's record from its files.
#[derive(Clone)]
pub struct Sync {
    pub db: Database,
    pub vocabulary: VocabularyHandle,
    pub project_dir: PathBuf,
}

impl Sync {
    /// Every page, logging how it went.
    pub async fn scan(&self) {
        let dir = crate::oxwiki_dir(&self.project_dir);
        match pages::scan_and_sync_all(&self.db, &self.vocabulary, &self.project_dir).await {
            Ok(report) if report.failures.is_empty() => {
                info!(dir = %dir.display(), synced = report.synced, "oxwiki scan complete");
            }
            Ok(report) => {
                let failed: Vec<&str> = report.failures.iter().map(|(s, _)| s.as_str()).collect();
                warn!(
                    synced = report.synced,
                    ?failed,
                    "oxwiki scan completed with per-page failures"
                );
            }
            Err(err) => warn!(?err, "oxwiki scan failed"),
        }
    }

    /// Each changed page's row; every page's when changes were lost.
    pub async fn batch(&self, batch: Batch) {
        if !batch.rescans.is_empty() {
            self.scan().await;
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
                pages::sync_page(&self.db, &self.vocabulary, &self.project_dir, slug).await
            {
                warn!(slug, ?err, "oxwiki page resync failed");
            }
        }
    }
}

/// Follows `changes` for as long as they come, syncing them while oxwiki is
/// present, and scans every file each time it appears (the publication that
/// first declares it, or turns it back on). Editors save a file in a few
/// rapid writes; one re-sync per page per burst is enough.
pub fn follow(changes: Subscription, sync: Sync, presence: Arc<OxwikiPresence>) {
    std::fs::create_dir_all(crate::oxwiki_dir(&sync.project_dir)).ok();
    let mut changes = changes;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = presence.appeared() => sync.scan().await,
                batch = changes.next_batch_after(Duration::from_millis(250)) => {
                    let Some(batch) = batch else { break };
                    if presence.present() {
                        sync.batch(batch).await;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::SqliteKnowledgePageStore;
    use oxplow_domain::knowledge::OXWIKI;
    use oxplow_domain::uri::build::knowledge_uri;

    /// A page's row follows its file, and when changes were lost every page's
    /// does, with no OS involved.
    #[tokio::test]
    async fn a_batch_restates_rows_and_a_rescan_restates_them_all() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().to_path_buf();
        let dir = crate::oxwiki_dir(&project);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::in_memory();
        let store = SqliteKnowledgePageStore::new(db.clone());
        let sync = Sync {
            db,
            vocabulary: VocabularyHandle::core(),
            project_dir: project,
        };
        std::fs::write(dir.join("one.md"), "# One\n").unwrap();
        std::fs::write(dir.join("two.md"), "# Two\n").unwrap();

        sync.batch(Batch {
            paths: vec![(
                dir.join("one.md"),
                oxplow_fs_watch::WatchEventKind::Modified,
            )],
            rescans: Vec::new(),
        })
        .await;
        let page = |slug: &str| knowledge_uri(OXWIKI, slug);
        assert!(store.body(&page("one")).await.unwrap().is_some());
        assert!(store.body(&page("two")).await.unwrap().is_none());

        sync.batch(Batch {
            paths: Vec::new(),
            rescans: vec![dir],
        })
        .await;
        assert!(store.body(&page("two")).await.unwrap().is_some());
    }
}
