//! The `wiki_page` row: a page's text (`body`) and what's derived from it,
//! written with its `.oxplow/wiki/<slug>.md` file in one run (P6.E2).
//! Search is the site index (`search_store`), which reads the row.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};

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

/// Write `page`'s row — with its `body` and that body's hash. The one writer of `wiki_page` — composes inside
/// `knowledge.write_page`'s transaction and the watcher's.
pub fn upsert_tx(
    conn: &rusqlite::Connection,
    page: &WikiPage,
    body: &str,
    body_hash: &str,
) -> Result<(), DomainError> {
    let json = |v: &Vec<String>| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO wiki_page (
            slug, title, body_path, body_excerpt, body_size_bytes,
            file_refs_json, related_notes_json, dir_refs_json,
            created_at, updated_at, body_hash, body
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(slug) DO UPDATE SET
            title = excluded.title,
            body_path = excluded.body_path,
            body_excerpt = excluded.body_excerpt,
            body_size_bytes = excluded.body_size_bytes,
            file_refs_json = excluded.file_refs_json,
            related_notes_json = excluded.related_notes_json,
            dir_refs_json = excluded.dir_refs_json,
            updated_at = excluded.updated_at,
            body_hash = excluded.body_hash,
            body = excluded.body",
        params![
            page.slug,
            page.title,
            page.body_path,
            page.body_excerpt,
            page.body_size_bytes,
            json(&page.file_refs),
            json(&page.related_notes),
            json(&page.dir_refs),
            ts_to_string(page.created_at),
            ts_to_string(page.updated_at),
            body_hash,
            body,
        ],
    )
    .map_err(crate::database::map_sql_err)?;
    Ok(())
}

/// Delete `slug`'s row; whether there was one.
pub fn delete_tx(conn: &rusqlite::Connection, slug: &str) -> Result<bool, DomainError> {
    let rows = conn
        .execute("DELETE FROM wiki_page WHERE slug = ?1", params![slug])
        .map_err(crate::database::map_sql_err)?;
    Ok(rows > 0)
}

/// `slug`'s row and the hash of the body it was written from, if any.
pub fn get_tx(
    conn: &rusqlite::Connection,
    slug: &str,
) -> Result<Option<(WikiPage, String)>, DomainError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT * FROM wiki_page WHERE slug = ?1",
        params![slug],
        |r| Ok((row_to_note(r)?, r.get::<_, String>("body_hash")?)),
    )
    .optional()
    .map_err(crate::database::map_sql_err)
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

    /// A page's title and full body, from its row (P6.E2: the body is
    /// written with the row, so readers never go to the file).
    pub async fn body(&self, slug: &str) -> Result<Option<(String, String)>, DomainError> {
        let slug = slug.to_string();
        self.db
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT title, body FROM wiki_page WHERE slug = ?1",
                    params![slug],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Upsert {
        async fn upsert(&self, page: &WikiPage) -> Result<(), DomainError>;
    }

    impl Upsert for SqliteWikiPageStore {
        async fn upsert(&self, page: &WikiPage) -> Result<(), DomainError> {
            let page = page.clone();
            self.db
                .transaction(move |tx| upsert_tx(tx, &page, "", ""))
                .await
        }
    }

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
}
