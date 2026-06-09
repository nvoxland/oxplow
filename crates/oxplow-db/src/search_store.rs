//! Unified site-wide search index (FTS5 / BM25).
//!
//! One standalone FTS5 table (`search_fts`) holds the searchable text for
//! every kind of entity — tasks, comments, notes, wiki pages, file contents
//! — paired with a `search_entry` identity table that maps
//! `(kind, ref_id, stream_id)` to the FTS rowid so an entity can be updated
//! or removed in place. Ranking is FTS5's built-in `bm25()` (title weighted
//! above body); snippets via `snippet()`.
//!
//! The index is written by the `Indexer` service in `oxplow-app`; this store
//! is the persistence layer it drives. See `.context/data-model.md`.

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::DomainError;

use crate::database::Database;

/// One ranked search result. `stream_id` is `None` for project-global
/// entities (wiki pages); `score` is the BM25 score (lower = better match).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SearchHit {
    pub kind: String,
    pub ref_id: String,
    pub stream_id: Option<String>,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Clone)]
pub struct SqliteSearchStore {
    db: Database,
}

/// Turn an arbitrary user query into a safe FTS5 MATCH expression: each
/// whitespace-separated token becomes a double-quoted prefix term
/// (`"tok"*`), joined by spaces (implicit AND). Quoting makes FTS5 operators
/// and punctuation literal, so junk input can't throw a syntax error, and the
/// trailing `*` makes every term a prefix match (our "fuzziness" for v1).
/// Returns an empty string when nothing searchable remains.
pub fn sanitize_query(raw: &str) -> String {
    raw.split_whitespace()
        .filter_map(|tok| {
            let cleaned: String = tok.chars().filter(|c| !c.is_control()).collect();
            let trimmed = cleaned.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Escape embedded double-quotes by doubling them (FTS5 string rule).
            let escaped = trimmed.replace('"', "\"\"");
            Some(format!("\"{escaped}\"*"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl SqliteSearchStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert or replace the indexed text for one entity, keyed by
    /// `(kind, ref_id, stream_id)`. `stream_id` is `None` for project-global
    /// entities.
    pub async fn upsert(
        &self,
        kind: &str,
        ref_id: &str,
        stream_id: Option<&str>,
        title: &str,
        body: &str,
    ) -> Result<(), DomainError> {
        let kind = kind.to_string();
        let ref_id = ref_id.to_string();
        let stream_id = stream_id.map(|s| s.to_string());
        let title = title.to_string();
        let body = body.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn
                    .transaction()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT rowid FROM search_entry \
                         WHERE kind = ?1 AND ref_id = ?2 \
                           AND COALESCE(stream_id, '') = COALESCE(?3, '')",
                        params![kind, ref_id, stream_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                let rowid = match existing {
                    Some(id) => {
                        tx.execute("DELETE FROM search_fts WHERE rowid = ?1", params![id])
                            .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                        id
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO search_entry (kind, ref_id, stream_id) \
                             VALUES (?1, ?2, ?3)",
                            params![kind, ref_id, stream_id],
                        )
                        .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                        tx.last_insert_rowid()
                    }
                };
                tx.execute(
                    "INSERT INTO search_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
                    params![rowid, title, body],
                )
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                tx.commit()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                Ok(())
            })
            .await
    }

    /// Remove one entity's index row. No-op if it isn't indexed.
    pub async fn remove(
        &self,
        kind: &str,
        ref_id: &str,
        stream_id: Option<&str>,
    ) -> Result<(), DomainError> {
        let kind = kind.to_string();
        let ref_id = ref_id.to_string();
        let stream_id = stream_id.map(|s| s.to_string());
        self.db
            .call_mut(move |conn| {
                let tx = conn
                    .transaction()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT rowid FROM search_entry \
                         WHERE kind = ?1 AND ref_id = ?2 \
                           AND COALESCE(stream_id, '') = COALESCE(?3, '')",
                        params![kind, ref_id, stream_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                if let Some(id) = existing {
                    tx.execute("DELETE FROM search_fts WHERE rowid = ?1", params![id])
                        .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                    tx.execute("DELETE FROM search_entry WHERE rowid = ?1", params![id])
                        .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                }
                tx.commit()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                Ok(())
            })
            .await
    }

    /// Drop every index row owned by a stream (called when a stream is
    /// archived/deleted so its file rows don't linger).
    pub async fn purge_stream(&self, stream_id: &str) -> Result<(), DomainError> {
        let stream_id = stream_id.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn
                    .transaction()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                tx.execute(
                    "DELETE FROM search_fts WHERE rowid IN \
                     (SELECT rowid FROM search_entry WHERE stream_id = ?1)",
                    params![stream_id],
                )
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                tx.execute(
                    "DELETE FROM search_entry WHERE stream_id = ?1",
                    params![stream_id],
                )
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                tx.commit()
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                Ok(())
            })
            .await
    }

    /// BM25-ranked search. `stream_id = Some(s)` returns rows scoped to `s`
    /// plus project-global rows; `None` searches everything. `kinds`, when
    /// non-empty, restricts to those entity kinds.
    pub async fn search(
        &self,
        query: &str,
        stream_id: Option<&str>,
        kinds: &[String],
        limit: usize,
    ) -> Result<Vec<SearchHit>, DomainError> {
        let match_query = sanitize_query(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let stream_id = stream_id.map(|s| s.to_string());
        let kinds: Vec<String> = kinds.to_vec();
        self.db
            .call(move |conn| {
                // Positional binds, in SQL order: match, stream_id (×2),
                // kinds…, limit.
                let mut sql = String::from(
                    "SELECT e.kind, e.ref_id, e.stream_id, f.title, \
                            snippet(search_fts, 1, '«', '»', '…', 16), \
                            bm25(search_fts, 5.0, 1.0) AS score \
                     FROM search_fts f \
                     JOIN search_entry e ON e.rowid = f.rowid \
                     WHERE search_fts MATCH ? \
                       AND (? IS NULL OR e.stream_id = ? OR e.stream_id IS NULL)",
                );
                let mut values: Vec<Value> = vec![
                    Value::Text(match_query),
                    stream_id.clone().map(Value::Text).unwrap_or(Value::Null),
                    stream_id.map(Value::Text).unwrap_or(Value::Null),
                ];
                if !kinds.is_empty() {
                    let placeholders = vec!["?"; kinds.len()].join(", ");
                    sql.push_str(&format!(" AND e.kind IN ({placeholders})"));
                    values.extend(kinds.into_iter().map(Value::Text));
                }
                sql.push_str(" ORDER BY score LIMIT ?");
                values.push(Value::Integer(limit as i64));

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params_from_iter(values), |row| {
                    Ok(SearchHit {
                        kind: row.get(0)?,
                        ref_id: row.get(1)?,
                        stream_id: row.get(2)?,
                        title: row.get(3)?,
                        snippet: row.get(4)?,
                        score: row.get(5)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> SqliteSearchStore {
        SqliteSearchStore::new(Database::in_memory())
    }

    #[tokio::test]
    async fn ranks_and_returns_matches() {
        let s = store().await;
        s.upsert(
            "task",
            "1",
            Some("s-a"),
            "Add login",
            "implement OAuth login flow",
        )
        .await
        .unwrap();
        s.upsert(
            "wiki",
            "auth",
            None,
            "Authentication",
            "how login and OAuth work",
        )
        .await
        .unwrap();
        let hits = s.search("login", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(hits.len(), 2, "both task and global wiki match");
        assert!(hits.iter().any(|h| h.kind == "task" && h.ref_id == "1"));
        assert!(hits.iter().any(|h| h.kind == "wiki" && h.ref_id == "auth"));
    }

    #[tokio::test]
    async fn prefix_matches_partial_token() {
        let s = store().await;
        s.upsert(
            "wiki",
            "auth",
            None,
            "Authentication",
            "authentication and authorization",
        )
        .await
        .unwrap();
        let hits = s.search("auth", None, &[], 10).await.unwrap();
        assert_eq!(hits.len(), 1, "`auth` prefix-matches `authentication`");
    }

    #[tokio::test]
    async fn stream_filter_scopes_file_rows_but_keeps_global() {
        let s = store().await;
        s.upsert(
            "file",
            "src/a.rs",
            Some("s-a"),
            "src/a.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        s.upsert(
            "file",
            "src/b.rs",
            Some("s-b"),
            "src/b.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        s.upsert("wiki", "w", None, "Widgets", "all about the widget")
            .await
            .unwrap();
        let hits = s.search("widget", Some("s-a"), &[], 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.ref_id.as_str()).collect();
        assert!(ids.contains(&"src/a.rs"), "stream a's file is in scope");
        assert!(
            !ids.contains(&"src/b.rs"),
            "stream b's file is filtered out"
        );
        assert!(ids.contains(&"w"), "global wiki is always in scope");
    }

    #[tokio::test]
    async fn kind_filter_restricts_results() {
        let s = store().await;
        s.upsert("task", "tsk1", Some("s-a"), "widget task", "build the widget")
            .await
            .unwrap();
        s.upsert(
            "file",
            "src/a.rs",
            Some("s-a"),
            "src/a.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        let hits = s
            .search("widget", Some("s-a"), &["file".to_string()], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "file");
    }

    #[tokio::test]
    async fn upsert_replaces_in_place() {
        let s = store().await;
        s.upsert("task", "tsk1", Some("s-a"), "old title", "old body widget")
            .await
            .unwrap();
        s.upsert("task", "tsk1", Some("s-a"), "new title", "new body gadget")
            .await
            .unwrap();
        assert!(s
            .search("widget", Some("s-a"), &[], 10)
            .await
            .unwrap()
            .is_empty());
        let hits = s.search("gadget", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(hits.len(), 1, "exactly one row, not a duplicate");
        assert_eq!(hits[0].title, "new title");
    }

    #[tokio::test]
    async fn remove_drops_the_row() {
        let s = store().await;
        s.upsert("note", "n1", Some("s-a"), "", "ephemeral widget note")
            .await
            .unwrap();
        assert_eq!(
            s.search("widget", Some("s-a"), &[], 10)
                .await
                .unwrap()
                .len(),
            1
        );
        s.remove("note", "n1", Some("s-a")).await.unwrap();
        assert!(s
            .search("widget", Some("s-a"), &[], 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn purge_stream_drops_only_that_stream() {
        let s = store().await;
        s.upsert("file", "a.rs", Some("s-a"), "a.rs", "widget")
            .await
            .unwrap();
        s.upsert("file", "b.rs", Some("s-b"), "b.rs", "widget")
            .await
            .unwrap();
        s.purge_stream("s-a").await.unwrap();
        let hits = s.search("widget", None, &[], 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].stream_id.as_deref(), Some("s-b"));
    }

    #[tokio::test]
    async fn junk_query_does_not_error() {
        let s = store().await;
        s.upsert("wiki", "w", None, "T", "body").await.unwrap();
        // Punctuation-only / FTS5-operator input must not throw.
        for q in ["", "   ", ":::", "\"", "AND OR NOT", "a-b (c)"] {
            assert!(
                s.search(q, None, &[], 10).await.is_ok(),
                "query {q:?} errored"
            );
        }
    }

    #[test]
    fn sanitize_quotes_and_prefixes_tokens() {
        assert_eq!(sanitize_query("foo bar"), "\"foo\"* \"bar\"*");
        assert_eq!(sanitize_query("  spaced   out "), "\"spaced\"* \"out\"*");
        assert_eq!(sanitize_query(""), "");
        // Embedded quote is escaped, not left to break the MATCH string.
        assert_eq!(sanitize_query("a\"b"), "\"a\"\"b\"*");
    }
}
