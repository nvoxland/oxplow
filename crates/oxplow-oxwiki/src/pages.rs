//! oxwiki's files to core's record: [`sync_page_tx`] restates one page from
//! `.oxplow/oxwiki/<slug>.md` — how a hand edit (or a scan) converges —
//! through the same record `oxplow.knowledge.write_page` writes, logged as
//! `system:oxwiki_watch`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use oxplow_db::knowledge_record::{
    body_hash, delete_page_tx, strip_body_version_literals, write_page_tx, PageWrite,
};
use oxplow_db::{Database, EventCtx, SqliteKnowledgePageStore};
use oxplow_domain::knowledge::OXWIKI;
use oxplow_domain::uri::build::knowledge_uri;
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{Anchors, DomainError, Timestamp};

use crate::{oxwiki_dir, page_path, valid_slug};

/// The source a sync is logged under: `system:oxwiki_watch`.
pub const SYNC_SOURCE: &str = "oxwiki_watch";

/// The file's mtime as a `Timestamp`, when the platform exposes one.
fn file_mtime(path: &Path) -> Option<Timestamp> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(Timestamp::from_unix_ms(i64::try_from(ms).ok()?))
}

/// Restate `slug` from its file. Links aren't refused here: whatever is on
/// disk is recorded. A body with `@version` literals is written back without
/// them (versions live on the edges); an unchanged body (its hash matches
/// the row's, as after a command's own write) is a no-op; a missing file
/// deletes the page. `updated_at` is the file's mtime, so a scan doesn't
/// reset every page's recency. A file whose name isn't a slug is not a
/// page: nothing is recorded for it, and a row it left is removed. Whether
/// anything changed.
pub fn sync_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    project_dir: &Path,
    slug: &str,
) -> Result<bool, DomainError> {
    let uri = knowledge_uri(OXWIKI, slug);
    if !valid_slug(slug) {
        return logged_delete(conn, ev, &uri);
    }
    let file_path = page_path(project_dir, slug);
    let raw = match fs::read_to_string(&file_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return logged_delete(conn, ev, &uri);
        }
        Err(e) => {
            return Err(DomainError::Storage(format!(
                "read oxwiki page {slug}: {e}"
            )))
        }
    };
    let body = strip_body_version_literals(&raw);
    if body != raw {
        if let Err(err) = fs::write(&file_path, &body) {
            tracing::warn!(
                slug,
                ?err,
                "failed to write back a version-stripped oxwiki page"
            );
        }
    }
    let stored = oxplow_db::knowledge_page_store::get_tx(conn, &uri)?;
    if stored.is_some_and(|(_, hash)| hash == body_hash(&body)) {
        return Ok(false);
    }
    let (_, env) = write_page_tx(
        conn,
        ev,
        &PageWrite {
            updated_at: file_mtime(&file_path).unwrap_or_else(Timestamp::now),
            ..PageWrite::new(OXWIKI, slug, &body)
        },
    )?;
    ev.append(conn, &env)?;
    Ok(true)
}

/// Delete `uri`'s record as the sync, logging it; whether there was a page.
fn logged_delete(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    uri: &str,
) -> Result<bool, DomainError> {
    match delete_page_tx(conn, ev, Anchors::default(), uri)? {
        Some(env) => {
            ev.append(conn, &env)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// [`sync_page_tx`] in its own transaction.
pub async fn sync_page(
    db: &Database,
    vocabulary: &VocabularyHandle,
    project_dir: &Path,
    slug: &str,
) -> Result<bool, DomainError> {
    let (vocabulary, project_dir, slug) = (
        vocabulary.clone(),
        project_dir.to_path_buf(),
        slug.to_string(),
    );
    db.transaction(move |tx| {
        let vocabulary = vocabulary.current();
        let ev = EventCtx::system(&vocabulary, SYNC_SOURCE);
        sync_page_tx(tx, &ev, &project_dir, &slug)
    })
    .await
}

/// Retry an operation through transient `SQLITE_BUSY` errors with
/// exponential backoff (50/100/200/400ms, then one final attempt). The
/// common hit: a scan racing a previous process still releasing the WAL
/// lock. Non-`Busy` errors return immediately.
pub async fn retry_busy<T, F, Fut>(mut op: F) -> Result<T, DomainError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, DomainError>>,
{
    let mut delay = std::time::Duration::from_millis(50);
    for _ in 0..4 {
        match op().await {
            Err(DomainError::Busy(err)) => {
                tracing::debug!(%err, ?delay, "storage busy — retrying");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            other => return other,
        }
    }
    op().await
}

/// What a full scan accomplished: pages synced (or pruned) and pages that
/// failed. A scan never stops at the first bad page.
#[derive(Debug, Default)]
pub struct ScanReport {
    pub synced: usize,
    pub failures: Vec<(String, DomainError)>,
}

/// Sync every `.md` file in oxwiki's folder and delete the recorded pages
/// whose file is gone. Per-page failures are collected into the report
/// (and warned); transient `Busy` errors retry with backoff first. The
/// outer `Err` is for listing the recorded pages failing.
pub async fn scan_and_sync_all(
    db: &Database,
    vocabulary: &VocabularyHandle,
    project_dir: &Path,
) -> Result<ScanReport, DomainError> {
    let dir = oxwiki_dir(project_dir);
    fs::create_dir_all(&dir).ok();
    let mut slugs: BTreeSet<String> = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
                slugs.insert(slug.to_string());
            }
        }
    }
    // A recorded page whose file is gone syncs to a delete.
    let pages = SqliteKnowledgePageStore::new(db.clone());
    for page in retry_busy(|| pages.list_store(OXWIKI)).await? {
        slugs.insert(page.id);
    }
    let mut report = ScanReport::default();
    for slug in &slugs {
        match retry_busy(|| sync_page(db, vocabulary, project_dir, slug)).await {
            Ok(_) => report.synced += 1,
            Err(err) => {
                tracing::warn!(slug, ?err, "oxwiki page sync failed during scan");
                report.failures.push((slug.clone(), err));
            }
        }
    }
    Ok(report)
}
