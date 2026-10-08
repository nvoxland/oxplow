//! Unified site-wide search index (FTS5 / BM25).
//!
//! One standalone FTS5 table (`search_fts`) holds the searchable text for
//! every kind of entity — tasks, comments, notes, wiki pages, file contents
//! — paired with a `search_entry` identity table that maps
//! `(kind, ref_id, stream_id)` to the FTS rowid so an entity can be updated
//! or removed in place. Ranking is FTS5's built-in `bm25()` (title weighted
//! above body); snippets via `snippet()`.
//!
//! Two writers drive it from `oxplow-app`: a kind indexed from a model
//! (tasks, comments, notes, wiki pages, an extension's searchable kind) is
//! restated by its asset (`kind_search`, [`restate_kind_tx`]: only the
//! entries that changed are written); file
//! contents are upserted by the `search.index` consumer (`indexer`). See
//! `.context/refs.md`.

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

/// One entry of a restated kind: its id, and its stream (`str1`) when it
/// belongs to one — `None` for a project-global entry.
pub type EntryKey = (String, Option<String>);

/// Make `kind`'s entries `entries` (`(ref_id, stream)` → `(title,
/// body)`), in the caller's transaction: what a kind whose whole index is
/// derived from a model does on each recompute, and — with none — how a
/// kind leaves the index. Only what differs is written (tsk896): an entry
/// whose title and body hash as stored is left alone, a changed one is
/// rewritten in place, a new one added, a gone one removed.
pub fn restate_kind_tx(
    conn: &rusqlite::Connection,
    kind: &str,
    entries: &std::collections::BTreeMap<EntryKey, (String, String)>,
) -> Result<Restated, DomainError> {
    let sql = crate::database::map_sql_err;
    let mut stored: std::collections::HashMap<EntryKey, (i64, Option<String>)> = {
        let mut st = conn
            .prepare(
                "SELECT rowid, ref_id, stream_id, content_hash FROM search_entry WHERE kind = ?1",
            )
            .map_err(sql)?;
        let rows = st
            .query_map(params![kind], |r| {
                Ok(((r.get(1)?, r.get(2)?), (r.get(0)?, r.get(3)?)))
            })
            .map_err(sql)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql)?
    };
    let mut out = Restated::default();
    for ((ref_id, stream), (title, body)) in entries {
        let hash = entry_hash(title, body);
        match stored.remove(&(ref_id.clone(), stream.clone())) {
            Some((_, Some(old))) if old == hash => {}
            Some((rowid, _)) => {
                conn.execute(
                    "UPDATE search_entry SET content_hash = ?2 WHERE rowid = ?1",
                    params![rowid, hash],
                )
                .map_err(sql)?;
                conn.execute(
                    "UPDATE search_fts SET title = ?2, body = ?3 WHERE rowid = ?1",
                    params![rowid, title, body],
                )
                .map_err(sql)?;
                out.changed += 1;
            }
            None => {
                conn.execute(
                    "INSERT INTO search_entry (kind, ref_id, stream_id, content_hash) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![kind, ref_id, stream, hash],
                )
                .map_err(sql)?;
                conn.execute(
                    "INSERT INTO search_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
                    params![conn.last_insert_rowid(), title, body],
                )
                .map_err(sql)?;
                out.added += 1;
            }
        }
    }
    for (rowid, _) in stored.into_values() {
        conn.execute("DELETE FROM search_fts WHERE rowid = ?1", params![rowid])
            .map_err(sql)?;
        conn.execute("DELETE FROM search_entry WHERE rowid = ?1", params![rowid])
            .map_err(sql)?;
        out.removed += 1;
    }
    Ok(out)
}

/// What a restate wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Restated {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

impl Restated {
    /// Whether it wrote anything.
    pub fn wrote(&self) -> bool {
        self.added + self.changed + self.removed > 0
    }
}

/// An entry's title and body, hashed (length-prefixed, so the split is
/// part of it).
fn entry_hash(title: &str, body: &str) -> String {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    for part in [title, body] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:032x}", h.digest128())
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
            .transaction(move |tx| {
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT rowid FROM search_entry \
                         WHERE kind = ?1 AND ref_id = ?2 \
                           AND COALESCE(stream_id, '') = COALESCE(?3, '')",
                        params![kind, ref_id, stream_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(crate::database::map_sql_err)?;
                let rowid = match existing {
                    Some(id) => {
                        tx.execute("DELETE FROM search_fts WHERE rowid = ?1", params![id])
                            .map_err(crate::database::map_sql_err)?;
                        id
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO search_entry (kind, ref_id, stream_id) \
                             VALUES (?1, ?2, ?3)",
                            params![kind, ref_id, stream_id],
                        )
                        .map_err(crate::database::map_sql_err)?;
                        tx.last_insert_rowid()
                    }
                };
                tx.execute(
                    "INSERT INTO search_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
                    params![rowid, title, body],
                )
                .map_err(crate::database::map_sql_err)?;
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
            .transaction(move |tx| {
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT rowid FROM search_entry \
                         WHERE kind = ?1 AND ref_id = ?2 \
                           AND COALESCE(stream_id, '') = COALESCE(?3, '')",
                        params![kind, ref_id, stream_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(crate::database::map_sql_err)?;
                if let Some(id) = existing {
                    tx.execute("DELETE FROM search_fts WHERE rowid = ?1", params![id])
                        .map_err(crate::database::map_sql_err)?;
                    tx.execute("DELETE FROM search_entry WHERE rowid = ?1", params![id])
                        .map_err(crate::database::map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Drop a stream's file rows (called when a stream is archived, so
    /// its files don't linger). A kind indexed from a model follows the
    /// model instead (`restate_kind_tx`).
    pub async fn purge_stream_files(&self, stream_id: &str) -> Result<(), DomainError> {
        let stream_id = stream_id.to_string();
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "DELETE FROM search_fts WHERE rowid IN \
                     (SELECT rowid FROM search_entry WHERE kind = 'file' AND stream_id = ?1)",
                    params![stream_id],
                )
                .map_err(crate::database::map_sql_err)?;
                tx.execute(
                    "DELETE FROM search_entry WHERE kind = 'file' AND stream_id = ?1",
                    params![stream_id],
                )
                .map_err(crate::database::map_sql_err)?;
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
        s.upsert(
            "task",
            "tsk1",
            Some("s-a"),
            "widget task",
            "build the widget",
        )
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
    async fn purge_stream_files_drops_only_that_streams_files() {
        let s = store().await;
        s.upsert("file", "a.rs", Some("s-a"), "a.rs", "widget")
            .await
            .unwrap();
        s.upsert("file", "b.rs", Some("s-b"), "b.rs", "widget")
            .await
            .unwrap();
        // A model-derived kind's rows follow its model, not the purge.
        s.upsert("task", "tsk1", Some("s-a"), "widget task", "")
            .await
            .unwrap();
        s.purge_stream_files("s-a").await.unwrap();
        let hits = s.search("widget", None, &[], 10).await.unwrap();
        let mut got: Vec<(&str, Option<&str>)> = hits
            .iter()
            .map(|h| (h.kind.as_str(), h.stream_id.as_deref()))
            .collect();
        got.sort();
        assert_eq!(got, vec![("file", Some("s-b")), ("task", Some("s-a"))]);
    }

    /// tsk864, tsk896: a restate writes only what differs — the same
    /// entries write nothing, an edit to one of three rewrites that one,
    /// and none removes the kind.
    #[tokio::test]
    async fn a_restate_writes_only_the_entries_that_changed() {
        let s = store().await;
        let entries = |second: &str| {
            std::collections::BTreeMap::from([
                (
                    ("tsk1".to_string(), Some("str1".to_string())),
                    ("widget".to_string(), "body".to_string()),
                ),
                (
                    ("tsk2".to_string(), Some("str1".to_string())),
                    (second.to_string(), "body".to_string()),
                ),
                (
                    ("tsk3".to_string(), Some("str1".to_string())),
                    ("sprocket".to_string(), "body".to_string()),
                ),
            ])
        };
        let restate = |e: std::collections::BTreeMap<EntryKey, (String, String)>| {
            let db = s.db.clone();
            async move {
                db.transaction(move |tx| restate_kind_tx(tx, "task", &e))
                    .await
                    .unwrap()
            }
        };
        let r = |added, changed, removed| Restated {
            added,
            changed,
            removed,
        };
        assert_eq!(restate(entries("cog")).await, r(3, 0, 0));
        assert_eq!(
            restate(entries("cog")).await,
            r(0, 0, 0),
            "the same entries"
        );
        assert_eq!(restate(entries("gadget")).await, r(0, 1, 0));
        let hits = s.search("gadget", Some("str1"), &[], 10).await.unwrap();
        assert_eq!(
            (hits[0].kind.as_str(), hits[0].ref_id.as_str()),
            ("task", "tsk2")
        );
        assert!(s.search("cog", None, &[], 10).await.unwrap().is_empty());
        assert_eq!(restate(Default::default()).await, r(0, 0, 3));
        assert!(s.search("gadget", None, &[], 10).await.unwrap().is_empty());
    }

    /// Multi-word semantics: tokens AND together (each as a quoted
    /// prefix term), so a phrase whose words both appear in a body
    /// matches — and a query with one absent word doesn't.
    #[tokio::test]
    async fn multi_word_query_ands_terms_across_a_body() {
        let s = store().await;
        s.upsert(
            "wiki",
            "architecture-overview",
            None,
            "Architecture Overview",
            "the workspace isolation rule is a hard invariant",
        )
        .await
        .unwrap();
        let hits = s
            .search("workspace isolation", None, &[], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "both words present → match");
        assert_eq!(hits[0].ref_id, "architecture-overview");
        // Words may be non-adjacent too (AND, not phrase).
        assert_eq!(
            s.search("workspace invariant", None, &[], 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            s.search("workspace nonexistent", None, &[], 10)
                .await
                .unwrap()
                .is_empty(),
            "an absent term must fail the AND"
        );
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
