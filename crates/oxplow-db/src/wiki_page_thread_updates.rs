//! Per-thread "last touched" attribution for wiki pages.
//!
//! Wiki pages are global (one body per slug). Attribution is tracked
//! here so the rail's "Finished" list can surface only the pages the
//! *current* thread authored or revised. Backing table is
//! `wiki_page_thread_update` (see V1 migration).

use rusqlite::params;

use oxplow_domain::{DomainError, ThreadId, Timestamp};

use crate::database::Database;

#[derive(Debug, Clone, PartialEq)]
pub struct WikiPageThreadUpdate {
    pub thread_id: ThreadId,
    pub slug: String,
    pub last_seen_at: Timestamp,
}

#[derive(Clone)]
pub struct SqliteWikiPageThreadUpdateStore {
    db: Database,
}

impl SqliteWikiPageThreadUpdateStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Idempotent on (thread, slug) — last-write wins on `last_seen_at`.
    pub async fn touch(
        &self,
        thread: &ThreadId,
        slug: &str,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        let thread = *thread;
        let slug = slug.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO wiki_page_thread_update (thread_id, slug, last_seen_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(thread_id, slug) DO UPDATE SET
                       last_seen_at = excluded.last_seen_at",
                    params![thread.value(), slug, ts_to_string(at),],
                )?;
                Ok(())
            })
            .await
    }

    /// Recent slugs `thread` touched, newest first.
    pub async fn list_for_thread(
        &self,
        thread: &ThreadId,
        limit: usize,
    ) -> Result<Vec<WikiPageThreadUpdate>, DomainError> {
        let thread = *thread;
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT thread_id, slug, last_seen_at
                     FROM wiki_page_thread_update
                     WHERE thread_id = ?1
                     ORDER BY last_seen_at DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![thread.value(), limit as i64], |row| {
                    let tid: i64 = row.get("thread_id")?;
                    let slug: String = row.get("slug")?;
                    let ts: String = row.get("last_seen_at")?;
                    Ok((tid, slug, ts))
                })?;
                let mut out = Vec::new();
                for row in rows {
                    let (tid, slug, ts) = row?;
                    let last_seen_at = string_to_ts(&ts).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    out.push(WikiPageThreadUpdate {
                        thread_id: ThreadId::new(tid),
                        slug,
                        last_seen_at,
                    });
                }
                Ok(out)
            })
            .await
    }
}

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn touch_then_list() {
        let db = Database::in_memory();
        // Migrations need to run before the table exists. The
        // in-memory DB applies them in `Database::in_memory`.
        // Insert a thread + stream first to satisfy the FK.
        let stream_id = 1_i64;
        let thread_id = 1_i64;
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, working_pane, talking_pane,
                                      working_session_id, talking_session_id,
                                      created_at, updated_at)
                 VALUES (?,'primary','t','main','refs/heads/main','main','/tmp','','','','',
                         '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z')",
                params![stream_id],
            )?;
            c.execute(
                "INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target,
                                      resume_session_id, summary, created_at, updated_at)
                 VALUES (?,?,'t','active',0,'working','','','2026-01-01T00:00:00.000Z',
                         '2026-01-01T00:00:00.000Z')",
                params![thread_id, stream_id],
            )?;
            c.execute(
                "INSERT INTO wiki_page (slug, title, body_path, created_at, updated_at)
                 VALUES ('foo','Foo','/tmp/foo.md','2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let store = SqliteWikiPageThreadUpdateStore::new(db);
        let tid = ThreadId::new(thread_id);
        store
            .touch(&tid, "foo", Timestamp::from_unix_ms(1_700_000_000_000))
            .await
            .unwrap();
        let rows = store.list_for_thread(&tid, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug, "foo");
    }
}
