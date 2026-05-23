//! Wiki-note metadata + FTS5-backed body search.
//!
//! Note body lives on disk at `.oxplow/wiki/<slug>.md`. This store
//! holds the metadata row + an FTS5 search index synced from
//! the on-disk body via `resync`.

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::Database;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WikiPage {
    pub slug: String,
    pub title: String,
    pub body_path: String,
    pub body_excerpt: String,
    pub body_size_bytes: i64,
    pub file_refs: Vec<String>,
    pub dir_refs: Vec<String>,
    pub related_notes: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WikiPageSearchHit {
    pub slug: String,
    pub title: String,
    pub snippet: String,
    pub updated_at: Timestamp,
}

#[derive(Clone)]
pub struct SqliteWikiPageStore {
    db: Database,
}

impl SqliteWikiPageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
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

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<WikiPage> {
    let slug: String = row.get("slug")?;
    let title: String = row.get("title")?;
    let body_path: String = row.get("body_path")?;
    let body_excerpt: String = row.get("body_excerpt")?;
    let body_size_bytes: i64 = row.get("body_size_bytes")?;
    let file_refs_json: String = row.get("file_refs_json")?;
    let related_notes_json: String = row.get("related_notes_json")?;
    let dir_refs_json: String = row
        .get("dir_refs_json")
        .unwrap_or_else(|_| "[]".to_string());
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let file_refs: Vec<String> = serde_json::from_str(&file_refs_json).unwrap_or_default();
    let related_notes: Vec<String> = serde_json::from_str(&related_notes_json).unwrap_or_default();
    let dir_refs: Vec<String> = serde_json::from_str(&dir_refs_json).unwrap_or_default();
    Ok(WikiPage {
        slug,
        title,
        body_path,
        body_excerpt,
        body_size_bytes,
        file_refs,
        dir_refs,
        related_notes,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
    })
}

impl SqliteWikiPageStore {
    pub async fn list(&self) -> Result<Vec<WikiPage>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM wiki_page ORDER BY updated_at DESC")?;
                let rows = stmt.query_map([], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn get(&self, slug: &str) -> Result<Option<WikiPage>, DomainError> {
        let slug = slug.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT * FROM wiki_page WHERE slug = ?1")?;
                let mut rows = stmt.query_map(params![slug], row_to_note)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn upsert(&self, note: &WikiPage) -> Result<(), DomainError> {
        let note = note.clone();
        self.db
            .call(move |conn| {
                let file_refs_json =
                    serde_json::to_string(&note.file_refs).unwrap_or_else(|_| "[]".to_string());
                let related_notes_json =
                    serde_json::to_string(&note.related_notes).unwrap_or_else(|_| "[]".to_string());
                let dir_refs_json =
                    serde_json::to_string(&note.dir_refs).unwrap_or_else(|_| "[]".to_string());
                conn.execute(
                    "INSERT INTO wiki_page (
                        slug, title, body_path, body_excerpt, body_size_bytes,
                        file_refs_json, related_notes_json, dir_refs_json,
                        created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(slug) DO UPDATE SET
                        title = excluded.title,
                        body_path = excluded.body_path,
                        body_excerpt = excluded.body_excerpt,
                        body_size_bytes = excluded.body_size_bytes,
                        file_refs_json = excluded.file_refs_json,
                        related_notes_json = excluded.related_notes_json,
                        dir_refs_json = excluded.dir_refs_json,
                        updated_at = excluded.updated_at",
                    params![
                        note.slug,
                        note.title,
                        note.body_path,
                        note.body_excerpt,
                        note.body_size_bytes,
                        file_refs_json,
                        related_notes_json,
                        dir_refs_json,
                        ts_to_string(note.created_at),
                        ts_to_string(note.updated_at),
                    ],
                )?;
                // FTS5 mirror.
                conn.execute(
                    "DELETE FROM wiki_page_fts WHERE slug = ?1",
                    params![note.slug],
                )?;
                conn.execute(
                    "INSERT INTO wiki_page_fts (slug, title, body_excerpt) VALUES (?1, ?2, ?3)",
                    params![note.slug, note.title, note.body_excerpt],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delete(&self, slug: &str) -> Result<(), DomainError> {
        let slug = slug.to_string();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM wiki_page WHERE slug = ?1", params![slug])?;
                conn.execute("DELETE FROM wiki_page_fts WHERE slug = ?1", params![slug])?;
                Ok(())
            })
            .await
    }

    /// FTS5-backed full-text search over the body excerpt + title.
    pub async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchHit>, DomainError> {
        let query = query.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT n.slug, n.title, snippet(wiki_page_fts, 2, '<b>', '</b>', '…', 12) AS snippet,
                            n.updated_at
                     FROM wiki_page_fts f
                     JOIN wiki_page n ON n.slug = f.slug
                     WHERE wiki_page_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![query, limit as i64], |row| {
                    let slug: String = row.get(0)?;
                    let title: String = row.get(1)?;
                    let snippet: String = row.get(2)?;
                    let updated_at: String = row.get(3)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(WikiPageSearchHit {
                        slug,
                        title,
                        snippet,
                        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Glob-by-title for the lighter search_wiki_pages MCP tool.
    pub async fn search_titles(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPage>, DomainError> {
        let pattern = format!("%{}%", query);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM wiki_page WHERE title LIKE ?1 OR slug LIKE ?1 \
                     ORDER BY updated_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![pattern, limit as i64], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[async_trait]
pub trait WikiPageStore: Send + Sync {
    async fn list(&self) -> Result<Vec<WikiPage>, DomainError>;
    async fn get(&self, slug: &str) -> Result<Option<WikiPage>, DomainError>;
    async fn upsert(&self, note: &WikiPage) -> Result<(), DomainError>;
    async fn delete(&self, slug: &str) -> Result<(), DomainError>;
    async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchHit>, DomainError>;
    async fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<WikiPage>, DomainError>;
}

#[async_trait]
impl WikiPageStore for SqliteWikiPageStore {
    async fn list(&self) -> Result<Vec<WikiPage>, DomainError> {
        SqliteWikiPageStore::list(self).await
    }
    async fn get(&self, slug: &str) -> Result<Option<WikiPage>, DomainError> {
        SqliteWikiPageStore::get(self, slug).await
    }
    async fn upsert(&self, note: &WikiPage) -> Result<(), DomainError> {
        SqliteWikiPageStore::upsert(self, note).await
    }
    async fn delete(&self, slug: &str) -> Result<(), DomainError> {
        SqliteWikiPageStore::delete(self, slug).await
    }
    async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchHit>, DomainError> {
        SqliteWikiPageStore::search_bodies(self, query, limit).await
    }
    async fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<WikiPage>, DomainError> {
        SqliteWikiPageStore::search_titles(self, query, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn note(slug: &str, title: &str, excerpt: &str) -> WikiPage {
        WikiPage {
            slug: slug.into(),
            title: title.into(),
            body_path: format!(".oxplow/wiki/{slug}.md"),
            body_excerpt: excerpt.into(),
            body_size_bytes: excerpt.len() as i64,
            file_refs: vec![],
            dir_refs: vec![],
            related_notes: vec![],
            created_at: now(),
            updated_at: now(),
        }
    }

    #[tokio::test]
    async fn upsert_get_round_trips() {
        let store = SqliteWikiPageStore::new(Database::in_memory());
        let n = note("hello", "Hello world", "the quick brown fox");
        store.upsert(&n).await.unwrap();
        let got = store.get("hello").await.unwrap().unwrap();
        assert_eq!(got, n);
    }

    #[tokio::test]
    async fn fts_finds_body_terms() {
        let store = SqliteWikiPageStore::new(Database::in_memory());
        store
            .upsert(&note("a", "Cats and dogs", "cats are great pets"))
            .await
            .unwrap();
        store
            .upsert(&note("b", "Lizards", "reptilian friends"))
            .await
            .unwrap();
        let hits = store.search_bodies("cats", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "a");
    }

    #[tokio::test]
    async fn search_titles_glob() {
        let store = SqliteWikiPageStore::new(Database::in_memory());
        store.upsert(&note("a", "Streams", "")).await.unwrap();
        store.upsert(&note("b", "Threads", "")).await.unwrap();
        let hits = store.search_titles("Thread", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "b");
    }

    #[tokio::test]
    async fn delete_clears_fts_too() {
        let store = SqliteWikiPageStore::new(Database::in_memory());
        store.upsert(&note("a", "x", "find me")).await.unwrap();
        store.delete("a").await.unwrap();
        assert!(store.search_bodies("me", 10).await.unwrap().is_empty());
    }
}
