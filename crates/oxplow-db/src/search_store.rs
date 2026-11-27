//! Unified site-wide search index.
//!
//! Every searchable thing — a page, a command, a task, a knowledge page, a
//! file of a stream — is one entry: `search_entry` holds its identity
//! (`kind` — its resource, `oxplow/file` —, its URI, its stream, the
//! version it was written at), and two FTS5 tables hold its text under
//! the same rowid:
//!
//! - `search_fts(title, body)` — words (porter stemming, prefix index),
//!   ranked by `bm25()`, title above body; snippets via `snippet()`.
//! - `search_name(name)` — the trigram index of what a person types for it
//!   (a file's name, a page's title, a command's label): a word *inside* a
//!   name (`View` in `MarkdownView.tsx`), which a word index can't find.
//!
//! [`SqliteSearchStore::search`] finds candidates in both and ranks them
//! all on one scale ([`crate::search_rank`]).
//!
//! Every kind is written the same way: restated ([`restate_kind_tx`],
//! [`plan_restate_tx`] + [`apply_restate_tx`]) to the entries it has now,
//! only what differs written. A kind indexed from a dataset (tasks,
//! knowledge pages, pages, commands, an extension's searchable kind) is
//! restated by its asset (`kind_search`) as a whole; files are restated
//! one stream at a time from the stream's snapshot tree (`indexer`). See
//! `.context/resources.md`.

use std::collections::{BTreeMap, HashMap};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::DomainError;

use crate::database::Database;
use crate::search_rank::{self, Candidate};

/// One ranked search result: the URI it found
/// (`oxplow:oxplow/task?id=tsk7&provider=oxtask`), which the caller
/// opens the way it opens any URI, and its resource. `stream_id` is `None`
/// for project-global entities (knowledge pages). `tier` is how it matched
/// ([`crate::search_rank`]: 0 exact … 5 only in the body); `score` is its
/// BM25 score when its text matched (lower = better), else 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SearchHit {
    pub resource: String,
    pub uri: String,
    pub stream_id: Option<String>,
    pub title: String,
    pub snippet: String,
    pub tier: u8,
    pub score: f64,
}

/// The text of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryText {
    /// What a person types for it (a file's name, a page's title).
    pub name: String,
    /// Its title (a file's path).
    pub title: String,
    pub body: String,
}

impl EntryText {
    /// An entry named by its title.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            name: title.clone(),
            title,
            body: body.into(),
        }
    }

    /// An entry whose name isn't its title.
    pub fn named(
        name: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            body: body.into(),
        }
    }
}

#[derive(Clone)]
pub struct SqliteSearchStore {
    db: Database,
}

/// Turn an arbitrary user query into a safe FTS5 MATCH expression: each
/// whitespace-separated token becomes a double-quoted prefix term
/// (`"tok"*`), joined by spaces (implicit AND). Quoting makes FTS5 operators
/// and punctuation literal, so junk input can't throw a syntax error, and the
/// trailing `*` makes every term a prefix match.
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

/// What a restate of one kind changes, decided from versions alone: the
/// entries to write (new, or whose version differs from the stored one)
/// and the stored entries that are gone. Only the entries in `write`
/// need their text read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestatePlan {
    /// Each entry to write, its version and its stored rowid (`None` for a
    /// new one).
    pub write: Vec<(EntryKey, String, Option<i64>)>,
    /// The rowids of the stored entries that are gone.
    pub remove: Vec<i64>,
}

/// What it takes to make `kind`'s entries `versions` (`(ref_id, stream)` →
/// version), against what is stored: an entry whose version is as stored
/// is left alone. `stream`, when given, restates that stream's entries of
/// the kind only (a stream's files); else the whole kind.
pub fn plan_restate_tx(
    conn: &rusqlite::Connection,
    kind: &str,
    stream: Option<&str>,
    versions: &BTreeMap<EntryKey, String>,
) -> Result<RestatePlan, DomainError> {
    let sql = crate::database::map_sql_err;
    let mut stored: HashMap<EntryKey, (i64, Option<String>)> = {
        let mut st = conn
            .prepare(
                "SELECT rowid, uri, stream_id, version FROM search_entry \
                 WHERE kind = ?1 AND (?2 IS NULL OR stream_id = ?2)",
            )
            .map_err(sql)?;
        let rows = st
            .query_map(params![kind, stream], |r| {
                Ok(((r.get(1)?, r.get(2)?), (r.get(0)?, r.get(3)?)))
            })
            .map_err(sql)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql)?
    };
    let mut plan = RestatePlan::default();
    for (key, version) in versions {
        match stored.remove(key) {
            Some((_, Some(old))) if &old == version => {}
            Some((rowid, _)) => plan.write.push((key.clone(), version.clone(), Some(rowid))),
            None => plan.write.push((key.clone(), version.clone(), None)),
        }
    }
    plan.remove = stored.into_values().map(|(rowid, _)| rowid).collect();
    plan.remove.sort_unstable();
    Ok(plan)
}

/// Carry out `plan` for `kind`, `texts` holding the text of every entry it
/// writes.
pub fn apply_restate_tx(
    conn: &rusqlite::Connection,
    kind: &str,
    plan: &RestatePlan,
    texts: &BTreeMap<EntryKey, EntryText>,
) -> Result<Restated, DomainError> {
    let sql = crate::database::map_sql_err;
    let mut out = Restated::default();
    for (key, version, rowid) in &plan.write {
        let Some(text) = texts.get(key) else {
            return Err(DomainError::Invalid(format!(
                "search kind `{kind}`: no text read for `{}`",
                key.0
            )));
        };
        let rowid = match rowid {
            Some(rowid) => {
                conn.execute(
                    "UPDATE search_entry SET version = ?2 WHERE rowid = ?1",
                    params![rowid, version],
                )
                .map_err(sql)?;
                delete_text(conn, *rowid)?;
                out.changed += 1;
                *rowid
            }
            None => {
                conn.execute(
                    "INSERT INTO search_entry (kind, uri, stream_id, version) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![kind, key.0, key.1, version],
                )
                .map_err(sql)?;
                out.added += 1;
                conn.last_insert_rowid()
            }
        };
        insert_text(conn, rowid, text)?;
    }
    for rowid in &plan.remove {
        delete_text(conn, *rowid)?;
        conn.execute("DELETE FROM search_entry WHERE rowid = ?1", params![rowid])
            .map_err(sql)?;
        out.removed += 1;
    }
    Ok(out)
}

/// Write an entry's text into both indexes.
fn insert_text(
    conn: &rusqlite::Connection,
    rowid: i64,
    text: &EntryText,
) -> Result<(), DomainError> {
    let sql = crate::database::map_sql_err;
    conn.execute(
        "INSERT INTO search_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
        params![rowid, text.title, text.body],
    )
    .map_err(sql)?;
    conn.execute(
        "INSERT INTO search_name (rowid, name) VALUES (?1, ?2)",
        params![rowid, text.name],
    )
    .map_err(sql)?;
    Ok(())
}

/// Drop an entry's text from both indexes.
fn delete_text(conn: &rusqlite::Connection, rowid: i64) -> Result<(), DomainError> {
    let sql = crate::database::map_sql_err;
    conn.execute("DELETE FROM search_fts WHERE rowid = ?1", params![rowid])
        .map_err(sql)?;
    conn.execute("DELETE FROM search_name WHERE rowid = ?1", params![rowid])
        .map_err(sql)?;
    Ok(())
}

/// Make `kind`'s entries (in `stream`, when given) `entries`, in the
/// caller's transaction, each versioned by its text's hash: what a kind
/// whose feed declares no `version` does on each recompute, and — with
/// none — how a kind leaves the index. Only what differs is written.
pub fn restate_kind_tx(
    conn: &rusqlite::Connection,
    kind: &str,
    stream: Option<&str>,
    entries: &BTreeMap<EntryKey, EntryText>,
) -> Result<Restated, DomainError> {
    let versions = entry_versions(entries);
    let plan = plan_restate_tx(conn, kind, stream, &versions)?;
    apply_restate_tx(conn, kind, &plan, entries)
}

/// The version of each entry of a feed that declares none: its text's
/// hash.
pub fn entry_versions(entries: &BTreeMap<EntryKey, EntryText>) -> BTreeMap<EntryKey, String> {
    entries
        .iter()
        .map(|(key, text)| (key.clone(), entry_hash(text)))
        .collect()
}

impl RestatePlan {
    /// Nothing to write or remove.
    pub fn is_empty(&self) -> bool {
        self.write.is_empty() && self.remove.is_empty()
    }
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

/// An entry's text, hashed (each part length-prefixed, so the split is
/// part of it).
fn entry_hash(text: &EntryText) -> String {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    for part in [&text.name, &text.title, &text.body] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:032x}", h.digest128())
}

/// The most candidates the text index gives one search, best BM25 first.
const TEXT_CANDIDATES: usize = 500;
/// The most candidates the name index gives one search, shortest name
/// first.
const NAME_CANDIDATES: usize = 1000;

/// A candidate as read: its identity, text and how the text index scored
/// it.
struct Found {
    kind: String,
    uri: String,
    stream_id: Option<String>,
    name: String,
    title: String,
    snippet: String,
    bm25: Option<f64>,
}

impl SqliteSearchStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert or replace one entry, named by its title, keyed by
    /// `(kind, ref_id, stream_id)`. `stream_id` is `None` for
    /// project-global entities. A kind keeps its entries by restating them
    /// ([`restate_kind_tx`]); this writes one alone.
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
        let text = EntryText::new(title, body);
        self.db
            .transaction(move |tx| {
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT rowid FROM search_entry \
                         WHERE kind = ?1 AND uri = ?2 \
                           AND COALESCE(stream_id, '') = COALESCE(?3, '')",
                        params![kind, ref_id, stream_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(crate::database::map_sql_err)?;
                let rowid = match existing {
                    Some(id) => {
                        delete_text(tx, id)?;
                        id
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO search_entry (kind, uri, stream_id) \
                             VALUES (?1, ?2, ?3)",
                            params![kind, ref_id, stream_id],
                        )
                        .map_err(crate::database::map_sql_err)?;
                        tx.last_insert_rowid()
                    }
                };
                insert_text(tx, rowid, &text)
            })
            .await
    }

    /// Drop a stream's file rows (called when a stream is archived, so
    /// its files don't linger). A kind indexed from a dataset follows the
    /// dataset instead (`restate_kind_tx`).
    pub async fn purge_stream_files(&self, stream_id: &str) -> Result<(), DomainError> {
        let stream_id = stream_id.to_string();
        self.db
            .transaction(move |tx| {
                restate_kind_tx(
                    tx,
                    oxplow_domain::uri::resources::FILE,
                    Some(&stream_id),
                    &BTreeMap::new(),
                )
                .map(|_| ())
            })
            .await
    }

    /// The ranked search ([`crate::search_rank`]): candidates from the
    /// text index (each word a prefix of a word of the title or body) and
    /// from the name index (names holding each word), ranked on one scale,
    /// best first. Only files belong to a stream: `stream_id = Some(s)`
    /// keeps the files of `s` and every other entry, whatever stream it is
    /// in; `None` keeps every stream's files too. `kinds`, when non-empty,
    /// restricts to those resources (`oxplow/file`).
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
        let query = query.to_string();
        let words = search_rank::query_words(&query);
        let stream_id = stream_id.map(|s| s.to_string());
        let kinds: Vec<String> = kinds.to_vec();
        let found = self
            .db
            .call(move |conn| {
                // Shared by both reads: the stream and kind filters, as SQL
                // and its binds.
                let mut filter =
                    String::from(" AND (? IS NULL OR e.kind <> 'oxplow/file' OR e.stream_id = ?)");
                let mut filter_values: Vec<Value> = vec![
                    stream_id.clone().map(Value::Text).unwrap_or(Value::Null),
                    stream_id.map(Value::Text).unwrap_or(Value::Null),
                ];
                if !kinds.is_empty() {
                    filter.push_str(&format!(
                        " AND e.kind IN ({})",
                        vec!["?"; kinds.len()].join(", ")
                    ));
                    filter_values.extend(kinds.into_iter().map(Value::Text));
                }

                let mut by_rowid: HashMap<i64, Found> = HashMap::new();
                // The text index.
                let mut values = vec![Value::Text(match_query)];
                values.extend(filter_values.iter().cloned());
                values.push(Value::Integer(TEXT_CANDIDATES as i64));
                let mut stmt = conn.prepare(&format!(
                    "SELECT e.rowid, e.kind, e.uri, e.stream_id, f.title, \
                            COALESCE(n.name, f.title), \
                            snippet(search_fts, 1, '«', '»', '…', 16), \
                            bm25(search_fts, 5.0, 1.0) AS score \
                     FROM search_fts f \
                     JOIN search_entry e ON e.rowid = f.rowid \
                     LEFT JOIN search_name n ON n.rowid = e.rowid \
                     WHERE search_fts MATCH ?{filter} \
                     ORDER BY score LIMIT ?"
                ))?;
                let rows = stmt.query_map(params_from_iter(values), |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        Found {
                            kind: r.get(1)?,
                            uri: r.get(2)?,
                            stream_id: r.get(3)?,
                            title: r.get(4)?,
                            name: r.get(5)?,
                            snippet: r.get(6)?,
                            bm25: Some(r.get(7)?),
                        },
                    ))
                })?;
                for row in rows {
                    let (rowid, found) = row?;
                    by_rowid.insert(rowid, found);
                }

                // The name index: names holding every word (a word of three
                // characters or more by its trigrams; a shorter one by a
                // scan), those that start with the first word first, then
                // the shortest — so a short query's names aren't lost
                // among the many bodies that hold its letters.
                if let Some(first) = words.first() {
                    let likes = vec!["n.name LIKE ?"; words.len()].join(" AND ");
                    let mut values: Vec<Value> = words
                        .iter()
                        .map(|w| Value::Text(format!("%{w}%")))
                        .collect();
                    values.extend(filter_values.iter().cloned());
                    values.push(Value::Text(format!("{first}%")));
                    values.push(Value::Integer(NAME_CANDIDATES as i64));
                    let mut stmt = conn.prepare(&format!(
                        "SELECT e.rowid, e.kind, e.uri, e.stream_id, f.title, n.name \
                         FROM search_name n \
                         JOIN search_entry e ON e.rowid = n.rowid \
                         JOIN search_fts f ON f.rowid = n.rowid \
                         WHERE {likes}{filter} \
                         ORDER BY n.name LIKE ? DESC, length(n.name) LIMIT ?"
                    ))?;
                    let rows = stmt.query_map(params_from_iter(values), |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            Found {
                                kind: r.get(1)?,
                                uri: r.get(2)?,
                                stream_id: r.get(3)?,
                                title: r.get(4)?,
                                name: r.get(5)?,
                                snippet: String::new(),
                                bm25: None,
                            },
                        ))
                    })?;
                    for row in rows {
                        let (rowid, found) = row?;
                        by_rowid.entry(rowid).or_insert(found);
                    }
                }
                Ok(by_rowid.into_values().collect::<Vec<Found>>())
            })
            .await?;
        Ok(rank(&query, found, limit))
    }
}

/// Rank what both indexes found and keep the best `limit`.
fn rank(query: &str, found: Vec<Found>, limit: usize) -> Vec<SearchHit> {
    let mut ranked: Vec<(u8, Found)> = found
        .into_iter()
        .filter_map(|f| {
            let tier = search_rank::tier(query, &candidate(&f))?;
            Some((tier, f))
        })
        .collect();
    ranked.sort_by(|(ta, a), (tb, b)| {
        search_rank::compare((*ta, &candidate(a)), (*tb, &candidate(b)))
            .then_with(|| a.uri.cmp(&b.uri))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(tier, f)| SearchHit {
            resource: f.kind,
            uri: f.uri,
            stream_id: f.stream_id,
            title: f.title,
            snippet: f.snippet,
            tier,
            score: f.bm25.unwrap_or(0.0),
        })
        .collect()
}

fn candidate(f: &Found) -> Candidate<'_> {
    Candidate {
        name: &f.name,
        title: &f.title,
        id: uri_id(&f.uri),
        bm25: f.bm25,
    }
}

/// The `id` param of a URI (`…?id=tsk12&…`), when it has one.
fn uri_id(uri: &str) -> Option<&str> {
    let identity = uri.split('#').next().unwrap_or(uri);
    let (_, query) = identity.split_once('?')?;
    query.split('&').find_map(|pair| pair.strip_prefix("id="))
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
            "oxplow/knowledge",
            "auth",
            None,
            "Authentication",
            "how login and OAuth work",
        )
        .await
        .unwrap();
        let hits = s.search("login", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(hits.len(), 2, "both task and global knowledge page match");
        assert!(hits.iter().any(|h| h.uri == "1"));
        assert!(hits.iter().any(|h| h.uri == "auth"));
    }

    #[tokio::test]
    async fn prefix_matches_partial_token() {
        let s = store().await;
        s.upsert(
            "oxplow/knowledge",
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
            "oxplow/file",
            "src/a.rs",
            Some("s-a"),
            "src/a.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        s.upsert(
            "oxplow/file",
            "src/b.rs",
            Some("s-b"),
            "src/b.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        s.upsert(
            "oxplow/knowledge",
            "w",
            None,
            "Widgets",
            "all about the widget",
        )
        .await
        .unwrap();
        let hits = s.search("widget", Some("s-a"), &[], 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.uri.as_str()).collect();
        assert!(ids.contains(&"src/a.rs"), "stream a's file is in scope");
        assert!(
            !ids.contains(&"src/b.rs"),
            "stream b's file is filtered out"
        );
        assert!(
            ids.contains(&"w"),
            "a global knowledge page is always in scope"
        );
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
            "oxplow/file",
            "src/a.rs",
            Some("s-a"),
            "src/a.rs",
            "fn widget() {}",
        )
        .await
        .unwrap();
        let hits = s
            .search("widget", Some("s-a"), &["oxplow/file".to_string()], 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].resource, "oxplow/file");
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
    async fn purge_stream_files_drops_only_that_streams_files() {
        let s = store().await;
        s.upsert("oxplow/file", "a.rs", Some("s-a"), "a.rs", "widget")
            .await
            .unwrap();
        s.upsert("oxplow/file", "b.rs", Some("s-b"), "b.rs", "widget")
            .await
            .unwrap();
        // A dataset-derived kind's rows follow its dataset, not the purge.
        s.upsert("task", "tsk1", Some("s-a"), "widget task", "")
            .await
            .unwrap();
        s.purge_stream_files("s-a").await.unwrap();
        let hits = s.search("widget", None, &[], 10).await.unwrap();
        let mut got: Vec<(&str, Option<&str>)> = hits
            .iter()
            .map(|h| (h.resource.as_str(), h.stream_id.as_deref()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![("oxplow/file", Some("s-b")), ("task", Some("s-a"))]
        );
    }

    /// A feed with versions is planned from them alone: the same versions
    /// plan nothing, one changed version plans that one entry (the only
    /// text the caller then reads), and a gone entry is removed.
    #[tokio::test]
    async fn a_versioned_restate_plans_only_the_changed_entries() {
        let s = store().await;
        let key = |id: &str| (id.to_string(), Some("str1".to_string()));
        let versions = |second: &str| {
            std::collections::BTreeMap::from([
                (key("trn1"), "v1".to_string()),
                (key("trn2"), second.to_string()),
            ])
        };
        let texts = std::collections::BTreeMap::from([
            (key("trn1"), EntryText::new("one", "first turn")),
            (key("trn2"), EntryText::new("two", "second turn")),
        ]);
        let db = s.db.clone();
        let (first, texts) = (versions("v1"), texts.clone());
        db.transaction(move |tx| {
            let plan = plan_restate_tx(tx, "turn", None, &first)?;
            assert_eq!(plan.write.len(), 2);
            apply_restate_tx(tx, "turn", &plan, &texts)
        })
        .await
        .unwrap();
        let plan = |v: std::collections::BTreeMap<EntryKey, String>| {
            let db = s.db.clone();
            async move {
                db.transaction(move |tx| plan_restate_tx(tx, "turn", None, &v))
                    .await
                    .unwrap()
            }
        };
        assert_eq!(plan(versions("v1")).await, RestatePlan::default());
        let changed = plan(versions("v2")).await;
        assert_eq!(changed.write.len(), 1);
        assert_eq!(changed.write[0].0, key("trn2"));
        assert!(changed.write[0].2.is_some(), "rewritten in place");
        let gone = plan(std::collections::BTreeMap::from([(
            key("trn1"),
            "v1".to_string(),
        )]))
        .await;
        assert!(gone.write.is_empty());
        assert_eq!(gone.remove.len(), 1);
    }

    /// A restate writes only what differs — the same
    /// entries write nothing, an edit to one of three rewrites that one,
    /// and none removes the kind.
    #[tokio::test]
    async fn a_restate_writes_only_the_entries_that_changed() {
        let s = store().await;
        let entries = |second: &str| {
            std::collections::BTreeMap::from([
                (
                    ("tsk1".to_string(), Some("str1".to_string())),
                    EntryText::new("widget", "body"),
                ),
                (
                    ("tsk2".to_string(), Some("str1".to_string())),
                    EntryText::new(second, "body"),
                ),
                (
                    ("tsk3".to_string(), Some("str1".to_string())),
                    EntryText::new("sprocket", "body"),
                ),
            ])
        };
        let restate = |e: std::collections::BTreeMap<EntryKey, EntryText>| {
            let db = s.db.clone();
            async move {
                db.transaction(move |tx| restate_kind_tx(tx, "task", None, &e))
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
            (hits[0].resource.as_str(), hits[0].uri.as_str()),
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
            "oxplow/knowledge",
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
        assert_eq!(hits[0].uri, "architecture-overview");
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
        s.upsert("oxplow/knowledge", "w", None, "T", "body")
            .await
            .unwrap();
        // Punctuation-only / FTS5-operator input must not throw.
        for q in ["", "   ", ":::", "\"", "AND OR NOT", "a-b (c)"] {
            assert!(
                s.search(q, None, &[], 10).await.is_ok(),
                "query {q:?} errored"
            );
        }
    }

    /// Restate one stream's files: each named by its file name, titled by
    /// its path.
    async fn files(s: &SqliteSearchStore, stream: &str, files: &[(&str, &str)]) {
        let entries: std::collections::BTreeMap<EntryKey, EntryText> = files
            .iter()
            .map(|(path, body)| {
                let name = path.rsplit('/').next().unwrap_or(path);
                (
                    (path.to_string(), Some(stream.to_string())),
                    EntryText::named(name, *path, *body),
                )
            })
            .collect();
        let stream = stream.to_string();
        s.db.transaction(move |tx| restate_kind_tx(tx, "oxplow/file", Some(&stream), &entries))
            .await
            .unwrap();
    }

    fn uris(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.uri.as_str()).collect()
    }

    /// The one ranking: a file named by the query beats a task titled with
    /// it in a longer name, and a path whose letters only appear scattered
    /// (d…e…v) isn't found at all.
    #[tokio::test]
    async fn a_name_match_comes_first_and_scattered_letters_are_no_match() {
        let s = store().await;
        files(
            &s,
            "s-a",
            &[
                ("apps/desktop/public/fonts/InterVariable.woff2", ""),
                ("DEV.md", "how to run from source"),
                ("docs/notes.md", "the dev loop"),
            ],
        )
        .await;
        s.upsert("task", "tsk1", None, "Dev tools need a home", "")
            .await
            .unwrap();
        let hits = s.search("dev", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(uris(&hits), vec!["DEV.md", "tsk1", "docs/notes.md"]);
        assert_eq!(
            hits.iter().map(|h| h.tier).collect::<Vec<_>>(),
            vec![1, 1, 5]
        );
    }

    /// A word inside a name (not at a word's start) is found from three
    /// characters, by the name index — the text index only sees whole
    /// words.
    #[tokio::test]
    async fn a_word_inside_a_name_is_found() {
        let s = store().await;
        files(
            &s,
            "s-a",
            &[("src/MarkdownView.tsx", ""), ("src/Other.tsx", "")],
        )
        .await;
        let hits = s.search("down", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(uris(&hits), vec!["src/MarkdownView.tsx"]);
        assert_eq!(hits[0].tier, 3);
        let hits = s.search("view", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(uris(&hits), vec!["src/MarkdownView.tsx"]);
        assert_eq!(hits[0].tier, 2);
    }

    /// One or two characters match the start of a word.
    #[tokio::test]
    async fn a_short_query_matches_word_starts() {
        let s = store().await;
        files(&s, "s-a", &[("DEV.md", ""), ("src/model.rs", "")]).await;
        let hits = s.search("de", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(uris(&hits), vec!["DEV.md"]);
    }

    /// An exact id is tier zero.
    #[tokio::test]
    async fn an_exact_id_comes_first() {
        let s = store().await;
        s.upsert(
            "oxplow/task",
            "oxplow:oxplow/task?id=tsk12&provider=oxtask",
            None,
            "Fix it",
            "tsk12 and more",
        )
        .await
        .unwrap();
        let hits = s.search("tsk12", None, &[], 10).await.unwrap();
        assert_eq!(hits[0].tier, 0);
    }

    /// Only files belong to a stream: a search for one stream still finds
    /// another stream's task.
    #[tokio::test]
    async fn a_streams_search_keeps_other_streams_entries_but_not_their_files() {
        let s = store().await;
        files(&s, "s-b", &[("widget.rs", "")]).await;
        s.upsert("oxplow/task", "tsk1", Some("s-b"), "Widget task", "")
            .await
            .unwrap();
        let hits = s.search("widget", Some("s-a"), &[], 10).await.unwrap();
        assert_eq!(uris(&hits), vec!["tsk1"]);
    }

    /// A short query finds names by their start even when thousands of
    /// bodies hold the letters: the name index serves it too.
    #[tokio::test]
    async fn a_short_query_finds_a_name_over_many_bodies() {
        let s = store().await;
        let many: Vec<(String, String)> = (0..3000)
            .map(|i| {
                (
                    format!("src/f{i}.rs"),
                    "de deep design decides delta ".repeat(20),
                )
            })
            .collect();
        let mut all: Vec<(&str, &str)> =
            many.iter().map(|(p, b)| (p.as_str(), b.as_str())).collect();
        all.push(("DEV.md", ""));
        files(&s, "s-a", &all).await;
        let hits = s.search("de", Some("s-a"), &[], 5).await.unwrap();
        assert_eq!(hits[0].uri, "DEV.md");
    }

    /// A restate of one stream's files leaves another stream's alone.
    #[tokio::test]
    async fn a_streams_restate_leaves_other_streams_files() {
        let s = store().await;
        files(&s, "s-a", &[("a.rs", "widget")]).await;
        files(&s, "s-b", &[("b.rs", "widget")]).await;
        files(&s, "s-a", &[("a2.rs", "widget")]).await;
        let hits = s.search("widget", None, &[], 10).await.unwrap();
        let mut got = uris(&hits);
        got.sort();
        assert_eq!(got, vec!["a2.rs", "b.rs"]);
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
