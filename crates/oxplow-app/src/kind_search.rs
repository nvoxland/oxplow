//! Search resources whose rows come from a dataset (P9.D3;
//! `.context/resources.md` "Searchable resources"): every resource
//! `resource` lists with a `searchable` view — oxplow-foundation's and
//! another extension's (`searchable: <dataset>`) alike — has that view's rows
//! (`ref`, `title`, `body`, `stream_id`) in the site-wide index
//! (`search_fts`) under the resource, each by its URI. Files aren't a
//! dataset's: their input is snapshot blobs, so the `search.index` consumer
//! indexes them.
//!
//! It is **index-time ingestion, as an asset** — not a query-time UNION:
//! `search_fts` owns the text it ranks (BM25 and `snippet()` need it in
//! the index), and a UNION would run every extension's SQL on each
//! keystroke, unranked. The index of one kind is derived data whose
//! inputs are tables (the dataset's, followed through the lineage), so it
//! is a [`Materializer`]: recomputed in its own transaction when a commit
//! touches one of them — never in the writer's.
//!
//! [`Assets::sync_search_kinds`] keeps one per searchable resource in
//! `resource` (what the vocabulary pass restates): a new or changed one
//! is (re)registered, and one that is gone — its extension removed, its
//! `searchable:` dropped — leaves the index with its entries.
//!
//! A resource of an extension that doesn't ship with oxplow is bounded: at
//! most [`MAX_ROWS`] rows, each body cut at [`MAX_BODY`] bytes; a shipped
//! one's (`bundled_extensions::BUNDLED`) aren't. A view with a `stream_id`
//! column scopes each hit to its stream. A row whose
//! `ref` isn't a URI of the resource is skipped. A
//! recompute writes only the entries that changed, were added or went: a
//! feed with a `version` column is planned from its versions alone and
//! reads the title and body of the changed rows only (a kind of many
//! large rows, such as turns); one without is compared by each row's
//! title-and-body hash.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_db::search_store::EntryKey;
use oxplow_db::Database;
use oxplow_domain::DomainError;

use crate::assets::{Assets, Materializer, Recomputed};

/// The most rows of one kind the index holds.
pub const MAX_ROWS: usize = 20_000;
/// The most bytes of one row's body it indexes.
pub const MAX_BODY: usize = 16 * 1024;
/// The most changed rows a versioned recompute reads the text of in one
/// query.
const TEXT_CHUNK: usize = 500;

/// The asset of a resource's index: `search:<resource>`.
pub fn asset_name(kind: &str) -> String {
    format!("search:{kind}")
}

/// One searchable resource as `resource` and the dataset registry have it.
/// Any of it changing re-registers its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchableKind {
    /// The view its rows come from.
    view: String,
    /// The view's SQL as compiled: an edit of the dataset changes it — and
    /// what the index holds — without touching the tables (tsk851).
    sql: String,
    /// The tables behind the view.
    tables: Vec<String>,
    /// Its extension ships with oxplow: its rows aren't bounded, and its
    /// `seq` is trusted for incremental reads.
    shipped: bool,
    /// Its view has a `stream_id` column: each hit is that stream's.
    streamed: bool,
    /// The resource as declared: a row's `ref` is one of its URIs.
    decl: oxplow_domain::uri::resources::ResourceDecl,
    /// Its view has a `version` column: a recompute is planned from it.
    versioned: bool,
    /// Its view has an INTEGER `seq` column that only grows and is unique
    /// (a row id): a feed of rows that only appear (and whose text no insert
    /// elsewhere changes) can be read past the highest `seq` it indexed when
    /// the inputs saw inserts only, instead of whole. Core's only.
    incremental: bool,
}

/// The index of one resource.
struct KindSearchIndex {
    db: Database,
    asset: String,
    kind: String,
    spec: SearchableKind,
    /// The highest `seq` the index holds, once a whole read has found it:
    /// where an insert-only recompute reads on from.
    watermark: parking_lot::Mutex<Option<i64>>,
    /// How many feed rows the last recompute read.
    rows_read: std::sync::atomic::AtomicUsize,
}

impl KindSearchIndex {
    fn new(db: Database, kind: &str, spec: SearchableKind) -> Self {
        Self {
            db,
            asset: asset_name(kind),
            kind: kind.to_string(),
            spec,
            watermark: parking_lot::Mutex::new(None),
            rows_read: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Materializer for KindSearchIndex {
    fn asset(&self) -> &str {
        &self.asset
    }

    fn inputs(&self) -> Vec<String> {
        self.spec.tables.clone()
    }

    /// What it indexes beyond its tables' rows: the view's SQL.
    fn definition(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        Some(hex::encode(Sha256::digest(
            format!("{:?}", self.spec).as_bytes(),
        )))
    }

    /// Reads the feed and works out what changed on a read connection (no
    /// write lock), then takes the write lock only to write the changes: an
    /// index that is up to date writes nothing and never waits for a writer.
    async fn recompute(&self, full: bool) -> Result<Recomputed, DomainError> {
        // The plan is made against what was stored when it was read; the
        // write re-plans (cheap: no feed) and starts over if that no longer
        // fits the texts read.
        for _ in 0..RESTATE_ATTEMPTS {
            // Inserts only since the last recompute, and a feed whose rows
            // only appear: read on from the highest `seq` indexed.
            let since = if !full && self.spec.incremental {
                *self.watermark.lock()
            } else {
                None
            };
            let (kind, spec) = (self.kind.clone(), self.spec.clone());
            let prepared = self
                .db
                .read(move |tx| Feed::of(tx, &kind, &spec)?.prepare(since))
                .await?;
            self.rows_read
                .store(prepared.read, std::sync::atomic::Ordering::SeqCst);
            let count = prepared.versions.len();
            let row_count = (!prepared.partial).then_some(count as i64);
            if prepared.plan.is_empty() {
                self.advance(&prepared);
                return Ok(Recomputed {
                    row_count,
                    ..Recomputed::default()
                });
            }
            let kind = self.kind.clone();
            let written = prepared.clone();
            let landed = self
                .db
                .transaction_labeled(format!("search kind {kind}"), move |tx| {
                    written.commit(tx, &kind)
                })
                .await?;
            if landed {
                self.advance(&prepared);
                return Ok(Recomputed {
                    row_count,
                    ..Recomputed::default()
                });
            }
        }
        Err(DomainError::Storage(format!(
            "search kind `{}`: its index kept changing while it was restated",
            self.kind
        )))
    }
}

impl KindSearchIndex {
    /// Remember how far the feed has been indexed.
    fn advance(&self, prepared: &Prepared) {
        if !self.spec.incremental {
            return;
        }
        let mut mark = self.watermark.lock();
        *mark = match (prepared.partial, *mark, prepared.max_seq) {
            (true, Some(old), Some(new)) => Some(old.max(new)),
            (true, old, None) => old,
            (_, _, new) => new,
        };
    }
}

/// How often a restate starts over because the stored entries moved
/// between its read and its write.
const RESTATE_ATTEMPTS: usize = 5;

/// What a feed read says to write: every row's version, the plan against
/// the stored entries, and the text of each row the plan writes.
#[derive(Clone)]
struct Prepared {
    versions: BTreeMap<EntryKey, String>,
    plan: oxplow_db::search_store::RestatePlan,
    texts: BTreeMap<EntryKey, (String, String)>,
    /// Only the rows past a watermark were read: what isn't in `versions`
    /// is not gone, so nothing is removed.
    partial: bool,
    /// The highest `seq` among the rows read.
    max_seq: Option<i64>,
    /// How many feed rows were read.
    read: usize,
}

impl Prepared {
    /// Write the plan in `tx`. False: the stored entries moved since it was
    /// made and a text it needs was not read; read again.
    fn commit(&self, tx: &rusqlite::Connection, kind: &str) -> Result<bool, DomainError> {
        let mut plan = oxplow_db::search_store::plan_restate_tx(tx, kind, &self.versions)?;
        if self.partial {
            plan.remove.clear();
        }
        if plan
            .write
            .iter()
            .any(|(key, ..)| !self.texts.contains_key(key))
        {
            return Ok(false);
        }
        oxplow_db::search_store::apply_restate_tx(tx, kind, &plan, &self.texts)?;
        Ok(true)
    }
}

/// One resource's feed, read in a recompute's read transaction.
struct Feed<'a> {
    tx: &'a rusqlite::Connection,
    /// The resource (`oxplow/thread`).
    kind: &'a str,
    spec: &'a SearchableKind,
    /// A registry of the resource alone, which a row's URI must hold for.
    resources: oxplow_domain::uri::resources::ResourceRegistry,
}

impl<'a> Feed<'a> {
    fn of(
        tx: &'a rusqlite::Connection,
        kind: &'a str,
        spec: &'a SearchableKind,
    ) -> Result<Self, DomainError> {
        let mut resources = oxplow_domain::uri::resources::ResourceRegistry::default();
        resources
            .register(spec.decl.clone())
            .map_err(|e| DomainError::Invalid(format!("resource `{kind}`: {e}")))?;
        Ok(Self {
            tx,
            kind,
            spec,
            resources,
        })
    }

    /// Read the feed: whole, or (`since`) its rows past a `seq`.
    fn prepare(&self, since: Option<i64>) -> Result<Prepared, DomainError> {
        if self.spec.versioned {
            self.prepare_by_version(since)
        } else {
            self.prepare_by_text(since)
        }
    }

    /// The feed's `seq` as a column, or none.
    fn seq(&self) -> &'static str {
        if self.spec.incremental {
            "CAST(seq AS INTEGER)"
        } else {
            "NULL"
        }
    }

    /// The rows past `since`, as a clause.
    fn past(&self, since: Option<i64>) -> String {
        match since {
            Some(seq) if self.spec.incremental => format!(" WHERE seq > {seq}"),
            _ => String::new(),
        }
    }
}

impl Feed<'_> {
    /// A row's stream: its view's `stream_id`, when it has one; else the
    /// rows are project-global.
    fn stream(&self) -> &'static str {
        if self.spec.streamed {
            "CAST(stream_id AS TEXT)"
        } else {
            "NULL"
        }
    }

    fn view(&self) -> String {
        format!("\"{}\"", self.spec.view.replace('"', "\"\""))
    }

    /// The bound on its rows: none for a shipped extension's.
    fn limit(&self) -> String {
        if self.spec.shipped {
            String::new()
        } else {
            format!(" LIMIT {}", MAX_ROWS + 1)
        }
    }

    /// `(ref, stream)` of a row as an entry key: its URI, when it is one
    /// of the resource's and holds for its params — else nothing a hit
    /// could open.
    fn key(&self, reference: Option<String>, stream: Option<String>) -> Option<EntryKey> {
        let uri = oxplow_domain::uri::build::parse_resource(&reference?, self.kind).ok()?;
        self.resources.validate(&uri).ok()?;
        Some((uri.identity(), stream))
    }

    fn body(&self, body: Option<String>) -> String {
        let mut body = body.unwrap_or_default();
        if !self.spec.shipped && body.len() > MAX_BODY {
            let mut end = MAX_BODY;
            while !body.is_char_boundary(end) {
                end -= 1;
            }
            body.truncate(end);
        }
        body
    }

    fn warn_bounds(&self, rows: usize, skipped: usize) {
        if !self.spec.shipped && rows > MAX_ROWS {
            tracing::warn!(kind = %self.kind, view = %self.spec.view, "more than {MAX_ROWS} rows; the rest aren't searchable");
        }
        if skipped > 0 {
            tracing::warn!(kind = %self.kind, view = %self.spec.view, skipped, "rows whose `uri` isn't one of the resource's URIs weren't indexed");
        }
    }

    /// Every row's text, compared by its hash.
    fn prepare_by_text(&self, since: Option<i64>) -> Result<Prepared, DomainError> {
        let sql = format!(
            "SELECT CAST(uri AS TEXT), CAST(title AS TEXT), CAST(body AS TEXT), {}, {} FROM {}{}{}",
            self.stream(),
            self.seq(),
            self.view(),
            self.past(since),
            self.limit()
        );
        let mut st = self.tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
        type Row = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        );
        let rows: Vec<Row> = st
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(oxplow_db::map_sql_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(oxplow_db::map_sql_err)?;
        let total = rows.len();
        let max_seq = rows.iter().filter_map(|r| r.4).max();
        let mut entries: BTreeMap<EntryKey, (String, String)> = BTreeMap::new();
        let mut skipped = 0usize;
        for (reference, title, body, stream, _) in rows.into_iter().take(self.bound()) {
            match self.key(reference, stream) {
                Some(key) => {
                    entries.insert(key, (title.unwrap_or_default(), self.body(body)));
                }
                None => skipped += 1,
            }
        }
        self.warn_bounds(total, skipped);
        let versions = oxplow_db::search_store::entry_versions(&entries);
        let mut plan = oxplow_db::search_store::plan_restate_tx(self.tx, self.kind, &versions)?;
        let partial = since.is_some();
        if partial {
            plan.remove.clear();
        }
        Ok(Prepared {
            versions,
            plan,
            texts: entries,
            partial,
            max_seq,
            read: total,
        })
    }

    /// Every row's version, then the text of the changed rows only.
    fn prepare_by_version(&self, since: Option<i64>) -> Result<Prepared, DomainError> {
        let sql = format!(
            "SELECT CAST(uri AS TEXT), {}, CAST(version AS TEXT), {} FROM {}{}{}",
            self.stream(),
            self.seq(),
            self.view(),
            self.past(since),
            self.limit()
        );
        let mut st = self.tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
        type Row = (Option<String>, Option<String>, Option<String>, Option<i64>);
        let rows: Vec<Row> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(oxplow_db::map_sql_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(oxplow_db::map_sql_err)?;
        let total = rows.len();
        let max_seq = rows.iter().filter_map(|r| r.3).max();
        let mut versions: BTreeMap<EntryKey, String> = BTreeMap::new();
        let mut skipped = 0usize;
        for (reference, stream, version, _) in rows.into_iter().take(self.bound()) {
            match self.key(reference, stream) {
                Some(key) => {
                    versions.insert(key, version.unwrap_or_default());
                }
                None => skipped += 1,
            }
        }
        self.warn_bounds(total, skipped);
        let mut plan = oxplow_db::search_store::plan_restate_tx(self.tx, self.kind, &versions)?;
        let partial = since.is_some();
        if partial {
            plan.remove.clear();
        }
        let mut texts: BTreeMap<EntryKey, (String, String)> = BTreeMap::new();
        for chunk in plan.write.chunks(TEXT_CHUNK) {
            let refs: Vec<String> = chunk.iter().map(|((id, _), ..)| id.clone()).collect();
            let sql = format!(
                "SELECT CAST(uri AS TEXT), {}, CAST(title AS TEXT), CAST(body AS TEXT) \
                 FROM {} WHERE uri IN ({})",
                self.stream(),
                self.view(),
                vec!["?"; refs.len()].join(", ")
            );
            let mut st = self.tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
            let found = st
                .query_map(rusqlite::params_from_iter(&refs), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<
                    Vec<(
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                    )>,
                >>()
                .map_err(oxplow_db::map_sql_err)?;
            for (reference, stream, title, body) in found {
                if let Some(key) = self.key(reference, stream) {
                    texts.insert(key, (title.unwrap_or_default(), self.body(body)));
                }
            }
        }
        Ok(Prepared {
            versions,
            plan,
            texts,
            partial,
            max_seq,
            read: total,
        })
    }

    fn bound(&self) -> usize {
        if self.spec.shipped {
            usize::MAX
        } else {
            MAX_ROWS
        }
    }
}

/// The searchable resources `resource` lists: every one of them, and those
/// whose view is published, each with what decides its rows.
struct Searchable {
    /// Every resource `resource` says is searchable, its view published or not.
    declared: std::collections::BTreeSet<String>,
    /// Those whose view is published: what to index.
    ready: BTreeMap<String, SearchableKind>,
}

async fn searchable_kinds(db: &Database) -> Result<Searchable, DomainError> {
    // Each resource with its view's compiled SQL (none while the view
    // isn't published), whether the view has a `version` column, and its
    // params as declared.
    type Declared = (
        String,
        String,
        bool,
        Option<String>,
        bool,
        String,
        bool,
        bool,
        bool,
    );
    // The extensions that ship with oxplow, as a JSON list for the query.
    let shipped = serde_json::to_string(
        &crate::bundled_extensions::BUNDLED
            .iter()
            .map(|b| b.name)
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into());
    let kinds: Vec<Declared> = db
        .read(move |tx| {
            let mut st = tx
                .prepare(
                    "SELECT k.resource, k.searchable, \
                            k.extension IN (SELECT value FROM json_each(?1)), \
                            (SELECT sql FROM sqlite_master WHERE type = 'view' AND name = k.searchable), \
                            EXISTS (SELECT 1 FROM pragma_table_info(k.searchable) WHERE name = 'version'), \
                            k.params, k.additional, \
                            EXISTS (SELECT 1 FROM pragma_table_info(k.searchable) WHERE name = 'seq'), \
                            EXISTS (SELECT 1 FROM pragma_table_info(k.searchable) WHERE name = 'stream_id') \
                     FROM resource k WHERE k.searchable IS NOT NULL ORDER BY k.resource",
                )
                .map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([&shipped], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                })
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows)
        })
        .await?;
    let views: Vec<String> = kinds.iter().map(|(_, view, ..)| view.clone()).collect();
    let tables = crate::assets::tables_behind(db, &views).await?;
    let declared = kinds.iter().map(|(kind, ..)| kind.clone()).collect();
    let ready = kinds
        .into_iter()
        // A view the compiler hasn't published (yet, or any more) has
        // nothing to index; it registers once the registry lists it.
        .filter_map(
            |(kind, view, shipped, sql, versioned, params, additional, seq, streamed)| {
                let tables = tables.get(&view)?.clone();
                let decl = oxplow_domain::uri::resources::ResourceDecl {
                    params: serde_json::from_str(&params).ok()?,
                    additional,
                    ..oxplow_domain::uri::resources::ResourceDecl::keyed(&kind, "id")
                };
                Some((
                    kind,
                    SearchableKind {
                        view,
                        sql: sql?,
                        tables,
                        shipped,
                        streamed,
                        versioned,
                        incremental: seq && shipped,
                        decl,
                    },
                ))
            },
        )
        .collect();
    Ok(Searchable { declared, ready })
}

impl Assets {
    /// Keep one search index per searchable resource (see the module): new
    /// or changed ones (re)registered, gone ones stopped and their entries
    /// removed — also those of a resource that went while oxplow wasn't
    /// running (an `asset_state` row with nothing behind it). A resource
    /// still declared searchable whose view isn't published — at start,
    /// before the extension datasets compile again — keeps its entries:
    /// "not compiled yet" isn't "gone". Run at start and when `resource`
    /// or the dataset registry changes.
    pub async fn sync_search_kinds(&self) -> Result<(), DomainError> {
        let Searchable {
            declared,
            ready: wanted,
        } = searchable_kinds(self.db()).await?;
        let mut have = self.search_kinds.lock().await;
        for (kind, old) in have.clone() {
            if wanted.get(&kind) != Some(&old) {
                self.remove(&asset_name(&kind));
                have.remove(&kind);
            }
        }
        for (kind, spec) in &wanted {
            if have.contains_key(kind) {
                continue;
            }
            self.register(Arc::new(KindSearchIndex::new(
                self.db().clone(),
                kind,
                spec.clone(),
            )));
            have.insert(kind.clone(), spec.clone());
        }
        // What was indexed for a resource that is no longer searchable goes —
        // found wherever it is left: its state, its failure, or entries
        // with neither (a restate that committed after a cleanup, or a
        // process that died between the two), files aside (tsk853).
        let keep: Vec<String> = declared.into_iter().collect();
        self.db()
            .transaction(move |tx| {
                let mut st = tx
                    .prepare(
                        "SELECT substr(asset, 8) FROM asset_state WHERE asset LIKE 'search:%' \
                         UNION SELECT substr(asset, 8) FROM asset_failure WHERE asset LIKE 'search:%' \
                         UNION SELECT kind FROM search_entry",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let found: Vec<String> = st
                    .query_map([], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)?;
                // Files aren't a dataset's.
                let gone = found.iter().filter(|kind| {
                    !keep.contains(kind) && kind.as_str() != oxplow_domain::uri::resources::FILE
                });
                for kind in gone {
                    oxplow_db::search_store::restate_kind_tx(tx, kind, &BTreeMap::new())?;
                    for table in ["asset_state", "asset_failure"] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE asset = ?1"),
                            [asset_name(kind)],
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                    }
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector_runner::{self, Collectors};
    use oxplow_db::SqlCell;
    use std::path::Path;
    use std::time::Duration;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// `acme`: pull requests collected as `pr` rows, a `prs` dataset that
    /// titles them, a `found` dataset that makes them searchable, and the
    /// kind `acme/pr` (`searchable` as given).
    fn acme(root: &Path, searchable: &str) {
        write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            &format!(
                "manifest: 2
name: acme
intent: {{ purpose: PRs., origin: null, examples: [] }}
collectors:
  - id: prs
    runtime: starlark
    entry: prs.star
    entities:
      - {{ name: pr, key: n, columns: {{ n: text, title: text, body: text }} }}
datasets:
  - name: prs
    version: 1
    description: Pull requests.
    fields:
      - {{ name: uri, type: TEXT, doc: The ref. }}
      - {{ name: title, type: TEXT, doc: Its title. }}
  - name: found
    version: 1
    description: Pull requests, as search finds them.
    fields:
      - {{ name: uri, type: TEXT, doc: The ref. }}
      - {{ name: title, type: TEXT, doc: Its title. }}
      - {{ name: body, type: TEXT, doc: Its description. }}
resources:
  - name: pr
    label: Pull request
    params: {{ id: {{ required: true, pattern: '^\\d+$' }} }}
    resolve: prs
{searchable}    icon: git-pull-request
"
            ),
        );
        write(
            root,
            ".oxplow/extensions/acme/datasets/prs.sql",
            "SELECT CAST(uri('acme/pr', 'id', n) AS TEXT) AS uri, title FROM ref('pr')",
        );
        write(
            root,
            ".oxplow/extensions/acme/datasets/found.sql",
            "SELECT CAST(uri('acme/pr', 'id', n) AS TEXT) AS uri, title, body FROM ref('pr')",
        );
        write(
            root,
            ".oxplow/extensions/acme/pages/pr.yaml",
            "title: Pull request\nfor: acme/pr\ndefault: true\n\
             params: [{ name: uri }, { name: id }]\n\
             loaders:\n  open: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: open }\n",
        );
    }

    fn rows(root: &Path, rows: &str) {
        write(
            root,
            ".oxplow/extensions/acme/prs.star",
            &format!("def transform(input):\n    return {{\"entities\": {{\"pr\": {rows}}}}}\n"),
        );
    }

    /// Activate the extension written after setup: its `pr` entity is
    /// created empty, so the datasets that read it compile.
    async fn declare_entities(svc: &std::sync::Arc<crate::Services>) {
        crate::test_fixtures::activate(svc).await;
    }

    async fn collect(svc: &std::sync::Arc<crate::Services>) {
        // What was written since (the collector's script) is deployed first.
        crate::test_fixtures::activate_with_problems(svc).await;
        let root = svc.layout.project_dir.clone();
        collector_runner::run_collector(
            &Collectors::of(svc, &root),
            &oxplow_domain::run::RunContext::core(
                oxplow_domain::Actor::Human,
                oxplow_domain::run::Mode::External,
            ),
            "acme",
            "prs",
            collector_runner::RunTrigger::Manual,
        )
        .await
        .unwrap();
    }

    /// Search for `query` until it finds `want` refs of `acme/pr` (the
    /// index follows its tables after a quiet moment).
    async fn found(svc: &crate::Services, query: &str, want: &[&str]) -> Vec<(String, String)> {
        let mut hits = Vec::new();
        for _ in 0..100 {
            hits = svc
                .search_store
                .search(query, None, &["acme/pr".to_string()], 10)
                .await
                .unwrap()
                .into_iter()
                .map(|h| {
                    let id = oxplow_domain::uri::build::param_of(&h.uri, "acme/pr", "id").unwrap();
                    (id, h.title)
                })
                .collect();
            let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
            if ids == want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        hits
    }

    /// A searchable view with a `stream_id` column scopes each hit to that
    /// stream, whoever's extension it is.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_searchable_views_stream_id_scopes_its_hits() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        acme(&root, "    searchable: found\n");
        let manifest = root.join(".oxplow/extensions/acme/extension.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap().replace(
            "      - { name: body, type: TEXT, doc: Its description. }\n",
            "      - { name: body, type: TEXT, doc: Its description. }\n      - { name: stream_id, type: INT, doc: Its stream. }\n",
        );
        std::fs::write(&manifest, text).unwrap();
        write(
            &root,
            ".oxplow/extensions/acme/datasets/found.sql",
            "SELECT CAST(uri('acme/pr', 'id', n) AS TEXT) AS uri, title, body, CAST(1 AS INT) AS stream_id FROM ref('pr')",
        );
        rows(
            &root,
            r#"[{"n": "12", "title": "Widget frobnicator", "body": "Adds the frobnication of widgets."}]"#,
        );
        declare_entities(svc).await;
        collect(svc).await;
        crate::test_fixtures::activate_with_problems(svc).await;
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(50));
        crate::datasets_changed::spawn(
            svc.db.clone(),
            Arc::new(crate::datasets_changed::DatasetWatermarks::default()),
            svc.events.clone(),
            assets.clone(),
            svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        assert_eq!(found(svc, "widget", &["12"]).await.len(), 1);
        let hit = svc
            .search_store
            .search("frobnication", None, &[], 10)
            .await
            .unwrap();
        assert_eq!(hit[0].stream_id.as_deref(), Some("1"), "{hit:?}");
    }

    /// P9.D3: a searchable kind's dataset rows are in the site-wide index
    /// under the kind, following the dataset as it changes; a row whose ref
    /// isn't one of the kind's is skipped; a kind that stops being
    /// searchable (or goes) takes its entries with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_searchable_kinds_rows_are_found_and_follow_the_dataset() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        acme(&root, "    searchable: found\n");
        rows(
            &root,
            r#"[{"n": "12", "title": "Widget frobnicator", "body": "Adds the frobnication of widgets."}, {"n": "x1", "title": "Widget impostor", "body": "Not a pull request's id."}]"#,
        );
        declare_entities(svc).await;
        collect(svc).await;
        crate::test_fixtures::activate_with_problems(svc).await;
        let errors: Vec<String> = svc
            .listed_extensions(&root)
            .await
            .into_iter()
            .filter(|e| e.name == "acme")
            .flat_map(|e| e.errors.iter().map(|d| e.shown(d)).collect::<Vec<_>>())
            .collect();
        assert_eq!(errors, Vec::<String>::new());
        let listed = svc
            .sql
            .query_sql(
                "SELECT searchable FROM v_resource WHERE resource = 'acme/pr'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed.rows,
            vec![vec![SqlCell::Text("v_acme_found".into())]]
        );

        // The change loop, as boot runs it, over assets that settle fast.
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(50));
        crate::datasets_changed::spawn(
            svc.db.clone(),
            Arc::new(crate::datasets_changed::DatasetWatermarks::default()),
            svc.events.clone(),
            assets.clone(),
            svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        assert_eq!(
            found(svc, "widget", &["12"]).await,
            vec![("12".to_string(), "Widget frobnicator".to_string())],
            "the row whose ref isn't a pull request's is skipped"
        );
        // Found by its body too, with the hit's snippet from it.
        let by_body = svc
            .search_store
            .search("frobnication", None, &[], 10)
            .await
            .unwrap();
        assert_eq!(
            (
                by_body[0].resource.as_str(),
                by_body[0].uri.as_str(),
                by_body[0].stream_id.clone()
            ),
            ("acme/pr", "oxplow:acme/pr?id=12", None)
        );
        assert!(
            by_body[0].snippet.contains("frobnication"),
            "{:?}",
            by_body[0]
        );

        // The dataset's rows change: so does what is found.
        rows(
            &root,
            r#"[{"n": "13", "title": "Gadget polish", "body": "Shinier gadgets."}]"#,
        );
        collect(svc).await;
        assert_eq!(found(svc, "widget", &[]).await, vec![]);
        assert_eq!(
            found(svc, "gadget", &["13"]).await,
            vec![("13".to_string(), "Gadget polish".to_string())]
        );

        // Its dataset's SQL changes, its tables don't (tsk851): the index
        // follows the new SQL without waiting for a collector to write.
        write(
            &root,
            ".oxplow/extensions/acme/datasets/found.sql",
            "SELECT CAST(uri('acme/pr', 'id', n) AS TEXT) AS uri, CAST(title || ' (open)' AS TEXT) AS title, body FROM ref('pr')",
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(
            found(svc, "open", &["13"]).await,
            vec![("13".to_string(), "Gadget polish (open)".to_string())]
        );

        // No longer searchable: its entries leave.
        acme(&root, "");
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(found(svc, "gadget", &[]).await, vec![]);
        let state = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM asset_state WHERE asset = 'search:acme/pr'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(state, 0);
    }

    /// tsk854: when the change listener lags it missed some batches — one
    /// may have held the `resource` or `dataset` write — so it resyncs the
    /// registry as if those changed, not only the assets' inputs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lagged_listener_resyncs_the_searchable_kinds() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        acme(&root, "    searchable: found\n");
        rows(
            &root,
            r#"[{"n": "12", "title": "Widget frobnicator", "body": "Frobs."}]"#,
        );
        declare_entities(svc).await;
        collect(svc).await;
        crate::test_fixtures::activate_with_problems(svc).await;
        // Nothing heard the `resource` write: the listener missed it.
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(50));
        let mut lineage = crate::datasets_changed::Lineage::load(&svc.db)
            .await
            .unwrap();
        crate::datasets_changed::react(
            &svc.db,
            &mut lineage,
            &assets,
            &svc.event_pump,
            &crate::datasets_changed::Changed::Everything,
        )
        .await;
        assert_eq!(
            found(svc, "widget", &["12"]).await,
            vec![("12".to_string(), "Widget frobnicator".to_string())]
        );
    }

    /// tsk852: at start, extension datasets are dropped and compiled again,
    /// so for a moment a searchable kind's view isn't there. That is "not
    /// compiled yet", not "gone": its entries stay until `resource` stops
    /// saying it is searchable.
    #[tokio::test]
    async fn a_kind_whose_view_isnt_compiled_yet_keeps_its_entries() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        svc.search_store
            .upsert(
                "acme/pr",
                "oxplow:acme/pr?id=12",
                None,
                "Widget frobnicator",
                "",
            )
            .await
            .unwrap();
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO resource (resource, extension, label, params, additional, searchable) \
                     VALUES ('acme/pr', 'acme', 'Pull request', '{\"id\":{\"required\":true}}', 0, 'v_acme_found')",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms) \
                     VALUES ('search:acme/pr', '2026-01-01T00:00:00Z', 0, 1)",
                    [],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        Assets::new(svc.db.clone(), Duration::from_millis(50))
            .sync_search_kinds()
            .await
            .unwrap();
        let left: Vec<String> = svc
            .search_store
            .search("widget", None, &[], 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.resource)
            .collect();
        assert_eq!(left, vec!["acme/pr"]);
    }

    /// tsk853: what a gone kind left is found from the index and the
    /// failures too, not only `asset_state`: a restate that committed
    /// after the cleanup (or a process that died between the two) left
    /// entries with no state row, and a kind whose recomputes only ever
    /// failed has a failure row and nothing else.
    #[tokio::test]
    async fn a_gone_kinds_orphaned_entries_and_failures_leave() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        for (kind, id) in [
            ("acme/pr", "oxplow:acme/pr?id=12"),
            (
                "oxplow/knowledge",
                "oxplow:oxplow/knowledge?id=widgets&store=oxwiki",
            ),
            ("oxplow/task", "oxplow:oxplow/task?id=tsk7&provider=oxtask"),
        ] {
            svc.search_store
                .upsert(kind, id, None, "Widget", "")
                .await
                .unwrap();
        }
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO asset_failure (asset, failed_at, error) \
                     VALUES ('search:acme_bad', '2026-01-01T00:00:00Z', 'no such view')",
                    [],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        Assets::new(svc.db.clone(), Duration::from_millis(50))
            .sync_search_kinds()
            .await
            .unwrap();
        let mut left: Vec<String> = svc
            .search_store
            .search("widget", None, &[], 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.resource)
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["oxplow/knowledge", "oxplow/task"],
            "core's entries are untouched"
        );
        let failures = svc
            .db
            .read(|tx| {
                tx.query_row("SELECT count(*) FROM asset_failure", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(failures, 0);
    }

    /// A kind that went while oxplow wasn't running (its index recorded,
    /// no kind behind it) is cleared at the next start's sync.
    #[tokio::test]
    async fn a_kind_gone_while_oxplow_was_down_leaves_the_index_at_start() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        svc.search_store
            .upsert(
                "acme/pr",
                "oxplow:acme/pr?id=12",
                None,
                "Widget frobnicator",
                "",
            )
            .await
            .unwrap();
        svc.search_store
            .upsert(
                "oxplow/knowledge",
                "oxplow:oxplow/knowledge?id=widgets&store=oxwiki",
                None,
                "Widgets",
                "",
            )
            .await
            .unwrap();
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms) \
                     VALUES ('search:acme/pr', '2026-01-01T00:00:00Z', 0, 1)",
                    [],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        Assets::new(svc.db.clone(), Duration::from_millis(50))
            .sync_search_kinds()
            .await
            .unwrap();
        let left: Vec<String> = svc
            .search_store
            .search("widget", None, &[], 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.resource)
            .collect();
        assert_eq!(
            left,
            vec!["oxplow/knowledge"],
            "core's entries are untouched"
        );
    }
}

/// Core's kinds (tsk864): tasks, comments and knowledge pages,
/// indexed from `v_search_<kind>` by the same assets as an extension's kind.
#[cfg(test)]
mod core_tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use oxplow_domain::StreamId;

    use crate::assets::Assets;
    use crate::Services;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Services::in_memory(dir.path())
            .await
            .expect("in-memory services");
        (svc, dir)
    }

    /// The change loop, as boot runs it, over assets that settle fast.
    fn change_loop(svc: &Services) -> Assets {
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(30));
        crate::datasets_changed::spawn(
            svc.db.clone(),
            Arc::new(crate::datasets_changed::DatasetWatermarks::default()),
            svc.events.clone(),
            assets.clone(),
            svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        assets
    }

    /// Wait (up to 5 s) for `check` to hold.
    async fn eventually<F, Fut>(what: &str, check: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        assert!(
            crate::test_fixtures::until(Duration::from_secs(5), check).await,
            "never: {what}"
        );
    }

    /// The index's `(ref_id, stream)` entries of `kind`.
    async fn entries(svc: &Services, kind: &'static str) -> Vec<(String, Option<String>)> {
        svc.db
            .read(move |c| {
                let mut s = c
                    .prepare("SELECT uri, stream_id FROM search_entry WHERE kind = ?1 ORDER BY uri")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([kind], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// Whether searching `q` in `stream` finds a `kind` hit.
    async fn found(svc: &Services, q: &str, stream: Option<StreamId>, kind: &str) -> bool {
        let stream = stream.map(|s| s.to_string());
        svc.search_store
            .search(q, stream.as_deref(), &[], 10)
            .await
            .unwrap()
            .iter()
            .any(|h| h.resource == kind)
    }

    /// Run `name` as a person; its result.
    async fn run(svc: &Services, name: &str, input: serde_json::Value) -> serde_json::Value {
        svc.commands
            .run(&oxplow_domain::Actor::Human, name, input, true)
            .await
            .unwrap()
            .result
    }

    /// Knowledge pages (oxwiki, the extension): found when written by
    /// command, found by their new text after an edit, gone when deleted.
    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_pages_are_indexed() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Services::in_memory(dir.path()).await.unwrap();
        svc.streams.ensure_primary().await.unwrap();
        crate::test_fixtures::activate(&svc).await;
        change_loop(&svc);
        run(
            &svc,
            "oxwiki.page.write",
            serde_json::json!({ "id": "gears", "body": "# Gears\n\nThe sprocket turns.\n" }),
        )
        .await;
        eventually("the page is found", || {
            found(&svc, "sprocket", None, "oxplow/knowledge")
        })
        .await;
        run(
            &svc,
            "oxwiki.page.write",
            serde_json::json!({ "id": "gears", "body": "# Gears\n\nThe flange holds.\n" }),
        )
        .await;
        eventually("an edit is found", || {
            found(&svc, "flange", None, "oxplow/knowledge")
        })
        .await;
        run(
            &svc,
            "oxwiki.page.delete",
            serde_json::json!({ "id": "gears" }),
        )
        .await;
        eventually("the deleted page leaves", || async {
            entries(&svc, "oxplow/knowledge").await.is_empty()
        })
        .await;
    }

    /// A handoff is found by its body, under its recipient's stream.
    #[tokio::test]
    async fn a_handoff_is_found_by_its_body() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let from = crate::test_fixtures::new_thread(&svc, stream.id, "A").await;
        let to = crate::test_fixtures::new_thread(&svc, stream.id, "B").await;
        change_loop(&svc);
        run(
            &svc,
            crate::commands::handoff::SEND,
            serde_json::json!({
                "from_thread": from.id.to_string(),
                "to_thread": to.id.to_string(),
                "body": "the sprocket half is yours",
            }),
        )
        .await;
        eventually("the handoff is found", || {
            found(&svc, "sprocket", Some(stream.id), "oxplow/handoff")
        })
        .await;
    }

    /// Thread notes and comments: written, replied to, gone when deleted.
    #[tokio::test]
    async fn notes_and_comments_are_indexed() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let out = run(
            &svc,
            crate::commands::comment::ADD,
            serde_json::json!({
                "stream": stream.id.to_string(),
                "thread": thread.id.to_string(),
                "target": oxplow_domain::uri::build::file_uri("src/lib.rs"),
                "quote": "quoted text", "body": "this mentions the doohickey",
            }),
        )
        .await;
        let comment = out["comment"]["id"].as_str().unwrap().to_string();
        eventually("the comment is found", || {
            found(&svc, "doohickey", Some(stream.id), "oxplow/comment")
        })
        .await;
        assert!(
            found(&svc, "quoted", Some(stream.id), "oxplow/comment").await,
            "by its quote too"
        );
        run(
            &svc,
            crate::commands::comment::REPLY,
            serde_json::json!({ "comment": comment, "body": "and the thingamajig" }),
        )
        .await;
        eventually("a reply is found", || {
            found(&svc, "thingamajig", Some(stream.id), "oxplow/comment")
        })
        .await;
        run(
            &svc,
            crate::commands::comment::DELETE,
            serde_json::json!({ "comment": comment }),
        )
        .await;
        eventually("the deleted comment leaves", || async {
            entries(&svc, "oxplow/comment").await.is_empty()
        })
        .await;
    }

    /// Setup the commands don't offer: one batch of SQL, committed.
    async fn exec(svc: &Services, sql: &'static str) {
        svc.db
            .transaction(move |tx| {
                tx.execute_batch(sql).map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// A thread is found by its title — and still found closed, then
    /// archived.
    #[tokio::test]
    async fn a_thread_is_found_by_its_title_even_closed_and_archived() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread =
            crate::test_fixtures::new_thread(&svc, stream.id, "Recalibrate the capacitor").await;
        change_loop(&svc);
        eventually("found by its title", || {
            found(&svc, "capacitor", Some(stream.id), "oxplow/thread")
        })
        .await;
        run(
            &svc,
            crate::commands::thread::CLOSE,
            serde_json::json!({ "thread": thread.id.to_string() }),
        )
        .await;
        oxplow_domain::stores::ThreadStore::archive(&*svc.thread_store, &thread.id)
            .await
            .unwrap();
        // The archive restates the thread's index; give it the time the
        // other kinds' leave-tests take, then it must still be there.
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            found(&svc, "capacitor", Some(stream.id), "oxplow/thread").await,
            "closed and archived threads stay"
        );
    }

    /// A session is found by a file its tool calls touched or a command it
    /// ran, and by its name when it has none (harness and day); a closed
    /// session on an archived thread stays.
    #[tokio::test]
    async fn an_agent_session_is_found_by_what_it_touched() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let session = crate::test_fixtures::open_session(&svc, thread.id).await;
        exec(&svc, "").await;
        let id = session.value();
        svc.db
            .transaction(move |tx| {
                for (kind, path, detail) in [
                    ("edit", Some("src/zebra_module.rs"), None),
                    ("shell", None, Some("cargo nextest run --quokka")),
                ] {
                    tx.execute(
                        "INSERT INTO agent_tool_call (thread_id, tool, kind, path, detail, at, agent_session_id)
                         VALUES (?1, 'T', ?2, ?3, ?4, '2026-10-01T00:00:00.000000Z', ?5)",
                        rusqlite::params![thread.id.value(), kind, path, detail, id],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        eventually("found by a touched path", || {
            found(
                &svc,
                "zebra_module",
                Some(stream.id),
                "oxplow/agent_session",
            )
        })
        .await;
        eventually("found by a shell command", || {
            found(&svc, "quokka", Some(stream.id), "oxplow/agent_session")
        })
        .await;
        assert!(
            found(&svc, "claude", Some(stream.id), "oxplow/agent_session").await,
            "by its harness when it has no title"
        );
        exec(
            &svc,
            "UPDATE agent_session SET closed_at = '2026-10-02T00:00:00.000000Z', closed_reason = 'closed'",
        )
        .await;
        oxplow_domain::stores::ThreadStore::archive(&*svc.thread_store, &thread.id)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            found(
                &svc,
                "zebra_module",
                Some(stream.id),
                "oxplow/agent_session"
            )
            .await
        );
    }

    /// A turn is found by what was asked and by what was answered.
    #[tokio::test]
    async fn a_turn_is_found_by_its_prompt_and_its_answer() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let t = thread.id.value();
        svc.db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, answer, started_at, ended_at)
                     VALUES (?1, 'why does the wombat stall', 'the gnu was holding the lock',
                             '2026-10-01T00:00:00.000000Z', '2026-10-01T00:05:00.000000Z')",
                    [t],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        eventually("found by the prompt", || {
            found(&svc, "wombat", Some(stream.id), "oxplow/turn")
        })
        .await;
        assert!(
            found(&svc, "gnu", Some(stream.id), "oxplow/turn").await,
            "by the answer"
        );
    }

    /// A feed whose rows only appear is read on from the highest `seq` it
    /// indexed when the inputs saw inserts only: one new turn is one row
    /// read, and the index is what a whole restate makes. A rewrite (`full`)
    /// reads the whole feed again.
    #[tokio::test]
    async fn an_insert_only_recompute_reads_only_the_new_rows() {
        use crate::assets::Materializer;
        use std::sync::atomic::Ordering;
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        let t = thread.id.value();
        let decide = |prompt: &'static str| {
            let svc = svc.clone();
            async move {
                svc.db
                    .transaction(move |tx| {
                        tx.execute(
                            "INSERT INTO agent_turn (thread_id, prompt, answer, started_at, ended_at)
                             VALUES (?1, ?2, 'an answer', '2026-10-01T00:00:00.000000Z', '2026-10-01T00:05:00.000000Z')",
                            rusqlite::params![t, prompt],
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
        };
        for q in ["First question", "Second question", "Third question"] {
            decide(q).await;
        }
        let index = index_of(&svc, "oxplow/turn").await;
        index.recompute(true).await.unwrap();
        assert_eq!(index.rows_read.load(Ordering::SeqCst), 3, "a whole read");

        decide("Fourth question").await;
        index.recompute(false).await.unwrap();
        assert_eq!(
            index.rows_read.load(Ordering::SeqCst),
            1,
            "one new row, one row read"
        );
        let incremental = entries(&svc, "oxplow/turn").await;
        assert_eq!(incremental.len(), 4);

        // The index is what a whole restate makes, and a recompute with
        // nothing new reads nothing.
        index.recompute(false).await.unwrap();
        assert_eq!(index.rows_read.load(Ordering::SeqCst), 0);
        index.recompute(true).await.unwrap();
        assert_eq!(index.rows_read.load(Ordering::SeqCst), 4);
        assert_eq!(entries(&svc, "oxplow/turn").await, incremental);
    }

    /// A commit is found by its subject, its message, its author and a
    /// path it changed; it belongs to no stream.
    #[tokio::test]
    async fn a_commit_is_found_by_its_message_author_and_files() {
        let (svc, _dir) = services().await;
        svc.streams.ensure_primary().await.unwrap();
        change_loop(&svc);
        exec(
            &svc,
            "INSERT INTO git_commit (sha, author, email, committed_at, subject, body)
               VALUES ('abc123def', 'Marmaduke Quill', 'q@x', '2026-10-01T00:00:00.000000Z',
                       'Tighten the gasket', 'The narwhal leak is fixed.');
             INSERT INTO git_commit_file (sha, path, status) VALUES ('abc123def', 'src/okapi.rs', 'M');",
        )
        .await;
        eventually("found by its subject", || {
            found(&svc, "gasket", None, "oxplow/commit")
        })
        .await;
        for word in ["narwhal", "marmaduke", "okapi"] {
            assert!(found(&svc, word, None, "oxplow/commit").await, "{word}");
        }
    }

    /// A core dataset is named by its bare name in its URI
    /// (`oxplow:oxplow/dataset?dataset=thread`), an extension's by
    /// `owner/name`: both are indexed, none skipped.
    #[tokio::test]
    async fn a_core_dataset_is_found_by_its_bare_name() {
        let (svc, _dir) = services().await;
        svc.streams.ensure_primary().await.unwrap();
        change_loop(&svc);
        eventually("the core dataset is indexed", || async {
            entries(&svc, "oxplow/dataset")
                .await
                .iter()
                .any(|(uri, _)| uri == "oxplow:oxplow/dataset?dataset=thread")
        })
        .await;
    }

    /// The loop that restated the dataset index every second: its view read
    /// `asset_state`, which every recompute (this one's too) writes. Once
    /// the indexes settle, nothing restates them: no search index is
    /// recomputed again while nothing it indexes changes.
    #[tokio::test]
    async fn settled_indexes_are_not_restated_while_nothing_changes() {
        let (svc, _dir) = services().await;
        svc.streams.ensure_primary().await.unwrap();
        change_loop(&svc);
        eventually("the dataset index exists", || async {
            !entries(&svc, "oxplow/dataset").await.is_empty()
        })
        .await;
        let computed = |svc: Arc<Services>| async move {
            svc.db
                .read(|tx| {
                    let mut s = tx
                        .prepare(
                            "SELECT asset, computed_at FROM asset_state \
                             WHERE asset LIKE 'search:%' ORDER BY asset",
                        )
                        .map_err(oxplow_db::map_sql_err)?;
                    let rows = s
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                        .map_err(oxplow_db::map_sql_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok(rows)
                })
                .await
                .unwrap()
        };
        // Let the start-up recomputes finish.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let before = computed(svc.clone()).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let after = computed(svc.clone()).await;
        assert_eq!(before, after, "an index was restated with nothing changed");
    }

    /// No search index may read a table that the assets themselves write
    /// (`asset_state`, `asset_failure`, the index): its recompute would
    /// feed itself.
    #[tokio::test]
    async fn no_search_index_reads_what_the_assets_write() {
        let (svc, _dir) = services().await;
        svc.streams.ensure_primary().await.unwrap();
        let views: Vec<String> = svc
            .db
            .read(|tx| {
                let mut s = tx
                    .prepare("SELECT searchable FROM resource WHERE searchable IS NOT NULL")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert!(!views.is_empty());
        let tables = crate::assets::tables_behind(&svc.db, &views).await.unwrap();
        for (view, tables) in tables {
            for table in &tables {
                assert!(
                    !matches!(
                        table.as_str(),
                        "asset_state" | "asset_failure" | "search_entry" | "search_fts"
                    ),
                    "{view} reads {table}"
                );
            }
        }
    }

    /// The catalog of datasets changes when a definition does, not when an
    /// asset recomputes: a page that reads `v_dataset` (the data catalog,
    /// a dataset's definition, Dataset data) would run again on every
    /// recompute of anything. What an asset recorded is `v_asset`.
    #[tokio::test]
    async fn the_dataset_catalog_does_not_read_what_the_assets_write() {
        let (svc, _dir) = services().await;
        let views = vec!["v_dataset".to_string()];
        let tables = crate::assets::tables_behind(&svc.db, &views).await.unwrap();
        let behind = &tables["v_dataset"];
        assert!(!behind.is_empty());
        for table in behind {
            assert!(
                !matches!(table.as_str(), "asset_state" | "asset_failure"),
                "v_dataset reads {table}"
            );
        }
    }

    /// The index of `kind`, built as `sync_search_kinds` builds it.
    async fn index_of(svc: &Services, kind: &str) -> super::KindSearchIndex {
        let spec = super::searchable_kinds(&svc.db)
            .await
            .unwrap()
            .ready
            .remove(kind)
            .expect("a searchable kind");
        super::KindSearchIndex::new(svc.db.clone(), kind, spec)
    }
}
