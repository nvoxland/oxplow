//! The `knowledge_page` row: a knowledge page as core records it, whatever
//! store keeps it — its body and what's derived from it — keyed by its URI
//! (`oxplow:oxplow/knowledge?id=<id>&store=<store>`). Written inside the
//! write's transaction (`oxplow.knowledge.write_page`, oxwiki's watcher, a
//! provider's `knowledge.page.recorded`). Search is the site index
//! (`search_store`), which reads the row. A thread's last sight of a page is
//! `knowledge_page_touch`.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, ThreadId, Timestamp};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct KnowledgePage {
    pub uri: String,
    pub store: String,
    pub id: String,
    pub kind: Option<String>,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub excerpt: String,
    pub body_size_bytes: i64,
    pub file_refs: Vec<String>,
    pub dir_refs: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Write `page`'s row — with its `body` and that body's hash. The one
/// writer of `knowledge_page`; it keeps the row's `created_at`.
pub fn upsert_tx(
    conn: &rusqlite::Connection,
    page: &KnowledgePage,
    body: &str,
    body_hash: &str,
) -> Result<(), DomainError> {
    let json = |v: &Vec<String>| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO knowledge_page (
            uri, store, id, kind, title, url, author, excerpt, body_size_bytes,
            files_json, dirs_json, created_at, updated_at, body_hash, body
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(uri) DO UPDATE SET
            kind = excluded.kind,
            title = excluded.title,
            url = excluded.url,
            author = excluded.author,
            excerpt = excluded.excerpt,
            body_size_bytes = excluded.body_size_bytes,
            files_json = excluded.files_json,
            dirs_json = excluded.dirs_json,
            updated_at = excluded.updated_at,
            body_hash = excluded.body_hash,
            body = excluded.body",
        params![
            page.uri,
            page.store,
            page.id,
            page.kind,
            page.title,
            page.url,
            page.author,
            page.excerpt,
            page.body_size_bytes,
            json(&page.file_refs),
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

/// Delete `uri`'s row (its touches with it); whether there was one.
pub fn delete_tx(conn: &rusqlite::Connection, uri: &str) -> Result<bool, DomainError> {
    conn.execute(
        "DELETE FROM knowledge_page_touch WHERE uri = ?1",
        params![uri],
    )
    .map_err(crate::database::map_sql_err)?;
    let rows = conn
        .execute("DELETE FROM knowledge_page WHERE uri = ?1", params![uri])
        .map_err(crate::database::map_sql_err)?;
    Ok(rows > 0)
}

/// Delete every page `store` keeps (a removed store's), with their touches;
/// their URIs.
pub fn delete_store_tx(
    conn: &rusqlite::Connection,
    store: &str,
) -> Result<Vec<String>, DomainError> {
    let uris = conn
        .prepare("SELECT uri FROM knowledge_page WHERE store = ?1")
        .and_then(|mut s| {
            s.query_map(params![store], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(crate::database::map_sql_err)?;
    for uri in &uris {
        delete_tx(conn, uri)?;
    }
    Ok(uris)
}

/// `uri`'s row and the hash of the body it was written from, if any.
pub fn get_tx(
    conn: &rusqlite::Connection,
    uri: &str,
) -> Result<Option<(KnowledgePage, String)>, DomainError> {
    conn.query_row(
        "SELECT * FROM knowledge_page WHERE uri = ?1",
        params![uri],
        |r| Ok((row_to_page(r)?, r.get::<_, String>("body_hash")?)),
    )
    .optional()
    .map_err(crate::database::map_sql_err)
}

/// `uri`'s recorded title and body, if it's recorded.
pub fn body_tx(
    conn: &rusqlite::Connection,
    uri: &str,
) -> Result<Option<(String, String)>, DomainError> {
    conn.query_row(
        "SELECT title, body FROM knowledge_page WHERE uri = ?1",
        params![uri],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(crate::database::map_sql_err)
}

/// `thread` saw the page `uri` at `at` (it wrote or read it); last write
/// wins.
pub fn touch_tx(
    conn: &rusqlite::Connection,
    thread: ThreadId,
    uri: &str,
    at: Timestamp,
) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO knowledge_page_touch (thread_id, uri, last_seen_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(thread_id, uri) DO UPDATE SET
           last_seen_at = excluded.last_seen_at",
        params![thread.value(), uri, ts_to_string(at)],
    )
    .map_err(crate::database::map_sql_err)?;
    Ok(())
}

fn row_to_page(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgePage> {
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let list = |col: &str| -> rusqlite::Result<Vec<String>> {
        let text: String = row.get(col)?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    };
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    Ok(KnowledgePage {
        uri: row.get("uri")?,
        store: row.get("store")?,
        id: row.get("id")?,
        kind: row.get("kind")?,
        title: row.get("title")?,
        url: row.get("url")?,
        author: row.get("author")?,
        excerpt: row.get("excerpt")?,
        body_size_bytes: row.get("body_size_bytes")?,
        file_refs: list("files_json")?,
        dir_refs: list("dirs_json")?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
    })
}

/// Reads of the record, outside a write.
#[derive(Clone)]
pub struct SqliteKnowledgePageStore {
    db: Database,
}

impl SqliteKnowledgePageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Every recorded page of `store`, newest first.
    pub async fn list_store(&self, store: &str) -> Result<Vec<KnowledgePage>, DomainError> {
        let store = store.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM knowledge_page WHERE store = ?1 ORDER BY updated_at DESC",
                )?;
                let rows = stmt.query_map(params![store], row_to_page)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn get(&self, uri: &str) -> Result<Option<KnowledgePage>, DomainError> {
        let uri = uri.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT * FROM knowledge_page WHERE uri = ?1",
                    params![uri],
                    row_to_page,
                )
                .optional()
            })
            .await
    }

    /// A page's title and full body, from its row (the body is written
    /// with the row, so readers never go to the store).
    pub async fn body(&self, uri: &str) -> Result<Option<(String, String)>, DomainError> {
        let uri = uri.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT title, body FROM knowledge_page WHERE uri = ?1",
                    params![uri],
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

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn page(store: &str, id: &str) -> KnowledgePage {
        KnowledgePage {
            uri: oxplow_domain::uri::build::knowledge_uri(store, id),
            store: store.into(),
            id: id.into(),
            kind: Some("note".into()),
            title: format!("{id} title"),
            url: None,
            author: None,
            excerpt: "the quick brown fox".into(),
            body_size_bytes: 19,
            file_refs: vec!["src/lib.rs".into()],
            dir_refs: vec![],
            created_at: now(),
            updated_at: now(),
        }
    }

    /// Two stores each keep a page with the same id: two rows, each read
    /// back by its URI; deleting one store's pages leaves the other's.
    #[tokio::test]
    async fn two_stores_keep_the_same_id_apart() {
        let db = Database::in_memory();
        let (a, b) = (
            page("oxwiki", "architecture"),
            page("vault", "architecture"),
        );
        {
            let (a, b) = (a.clone(), b.clone());
            db.transaction(move |tx| {
                upsert_tx(tx, &a, "A", "ha")?;
                upsert_tx(tx, &b, "B", "hb")
            })
            .await
            .unwrap();
        }
        let store = SqliteKnowledgePageStore::new(db.clone());
        assert_eq!(store.get(&a.uri).await.unwrap(), Some(a.clone()));
        assert_eq!(
            store.body(&b.uri).await.unwrap(),
            Some(("architecture title".into(), "B".into()))
        );
        let gone = db
            .transaction(|tx| delete_store_tx(tx, "vault"))
            .await
            .unwrap();
        assert_eq!(gone, vec![b.uri.clone()]);
        assert_eq!(store.list_store("oxwiki").await.unwrap(), vec![a]);
        assert!(store.list_store("vault").await.unwrap().is_empty());
    }
}
