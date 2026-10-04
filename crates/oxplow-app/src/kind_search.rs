//! Search kinds whose rows come from a model (P9.D3, tsk864;
//! `.context/refs.md` "Searchable kinds"): a plugin kind declared
//! `searchable: <model>` has that model's rows (`ref`, `title`, `body`)
//! in the site-wide index (`search_fts`) under its kind, so the launcher
//! finds `[[pr:12]]` by its title; core's tasks, comments, thread notes and
//! wiki pages are indexed the same way from `v_search_<kind>`
//! ([`CORE_KINDS`]), each row with its stream. Files aren't: their input
//! is snapshot blobs, so the `search.index` consumer indexes them.
//!
//! It is **index-time ingestion, as an asset** — not a query-time UNION:
//! `search_fts` owns the text it ranks (BM25 and `snippet()` need it in
//! the index), and a UNION would run every extension's SQL on each
//! keystroke, unranked. The index of one kind is derived data whose
//! inputs are tables (the model's, followed through the lineage), so it
//! is a [`Materializer`]: recomputed, whole, in its own transaction when
//! a commit touches one of them — never in the writer's.
//!
//! [`Assets::sync_search_kinds`] keeps one per searchable kind in
//! `ref_kind` (what the vocabulary pass restates): a new or changed kind
//! is (re)registered, and a kind that is gone — its extension removed,
//! its `searchable:` dropped — leaves the index with its entries.
//!
//! A plugin kind is bounded: at most [`MAX_ROWS`] rows, each body cut at
//! [`MAX_BODY`] bytes; core's aren't. A row whose `ref` isn't one of the
//! kind's (`<kind>:<id>`, the id matching its pattern) is skipped. A
//! recompute writes only the entries whose title or body changed, were
//! added or went (`restate_kind_tx`, per-entry hashes; tsk896), so an edit
//! writes one entry and a start with nothing changed writes nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_db::Database;
use oxplow_domain::DomainError;
use rusqlite::OptionalExtension;

use crate::assets::{Assets, Materializer, Recomputed};

/// The most rows of one kind the index holds.
pub const MAX_ROWS: usize = 20_000;
/// The most bytes of one row's body it indexes.
pub const MAX_BODY: usize = 16 * 1024;

/// Core's kinds indexed from a model: the search kind, its view, the
/// canonical ref its rows carry up to the id (tsk922: a published model's
/// `ref` is a canonical ref, the one a hit opens), and the id. The view's
/// rows carry a `stream_id`.
pub const CORE_KINDS: &[(&str, &str, &str, &str)] = &[
    ("task", "v_search_task", "work_item:oxplow:", r"^tsk\d+$"),
    ("comment", "v_search_comment", "comment:", r"^cmt\d+$"),
    ("note", "v_search_note", "task_note:", r"^not\d+$"),
    ("wiki", "v_search_wiki", "wiki:", r"^.+$"),
];

/// The asset of a kind's index: `search:<kind>`.
pub fn asset_name(kind: &str) -> String {
    format!("search:{kind}")
}

/// One searchable kind as `ref_kind` and the model registry have it. Any
/// of it changing re-registers its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchableKind {
    /// The view its rows come from.
    view: String,
    /// The view's SQL as compiled: an edit of the model changes it — and
    /// what the index holds — without touching the tables (tsk851).
    sql: String,
    /// What a row's `ref` is up to its id: `<kind>:` for a plugin kind,
    /// the canonical ref's for core's (`work_item:oxplow:`).
    ref_prefix: String,
    id_pattern: String,
    /// The tables behind the view.
    tables: Vec<String>,
    /// Core's: its rows carry their stream and aren't bounded.
    core: bool,
}

/// The index of one kind.
struct KindSearchIndex {
    db: Database,
    asset: String,
    kind: String,
    spec: SearchableKind,
}

#[async_trait]
impl Materializer for KindSearchIndex {
    fn asset(&self) -> &str {
        &self.asset
    }

    fn inputs(&self) -> Vec<String> {
        self.spec.tables.clone()
    }

    async fn recompute(&self, _full: bool) -> Result<Recomputed, DomainError> {
        let (kind, view, core) = (self.kind.clone(), self.spec.view.clone(), self.spec.core);
        let spec_prefix = self.spec.ref_prefix.clone();
        let id = regex::Regex::new(&self.spec.id_pattern)
            .map_err(|e| DomainError::Invalid(format!("ref kind `{kind}`'s id pattern: {e}")))?;
        let indexed = self
            .db
            .transaction(move |tx| {
                let (stream, limit) = if core {
                    ("CAST(stream_id AS TEXT)", String::new())
                } else {
                    ("NULL", format!(" LIMIT {}", MAX_ROWS + 1))
                };
                let sql = format!(
                    "SELECT CAST(ref AS TEXT), CAST(title AS TEXT), CAST(body AS TEXT), {stream} \
                     FROM \"{}\"{limit}",
                    view.replace('"', "\"\""),
                );
                let mut st = tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
                type Row = (Option<String>, Option<String>, Option<String>, Option<String>);
                let rows: Vec<Row> = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)?;
                let bound = if core { usize::MAX } else { MAX_ROWS };
                if rows.len() > bound {
                    tracing::warn!(%kind, %view, "more than {MAX_ROWS} rows; the rest aren't searchable");
                }
                let prefix = spec_prefix.as_str();
                let mut entries: BTreeMap<oxplow_db::search_store::EntryKey, (String, String)> =
                    BTreeMap::new();
                let mut skipped = 0usize;
                for (r, title, body, stream) in rows.into_iter().take(bound) {
                    // One of the kind's refs, or it isn't something a hit
                    // could open.
                    let Some(ref_id) = r
                        .as_deref()
                        .and_then(|r| r.strip_prefix(prefix))
                        .filter(|id_text| id.is_match(id_text))
                    else {
                        skipped += 1;
                        continue;
                    };
                    let mut body = body.unwrap_or_default();
                    if !core && body.len() > MAX_BODY {
                        let mut end = MAX_BODY;
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        body.truncate(end);
                    }
                    entries.insert(
                        (ref_id.to_string(), stream),
                        (title.unwrap_or_default(), body),
                    );
                }
                if skipped > 0 {
                    tracing::warn!(%kind, %view, skipped, "rows whose `ref` isn't one of the kind's weren't indexed");
                }
                let count = entries.len();
                oxplow_db::search_store::restate_kind_tx(tx, &kind, &entries)?;
                Ok(count)
            })
            .await?;
        Ok(Recomputed {
            row_count: Some(indexed as i64),
            ..Recomputed::default()
        })
    }
}

/// The searchable kinds `ref_kind` lists: every one of them, and those
/// whose view is published, each with what decides its rows.
struct Searchable {
    /// Every kind `ref_kind` says is searchable, its view published or not.
    declared: std::collections::BTreeSet<String>,
    /// Those whose view is published: what to index.
    ready: BTreeMap<String, SearchableKind>,
}

async fn searchable_kinds(db: &Database) -> Result<Searchable, DomainError> {
    // Each kind with its view's compiled SQL (none while the view isn't
    // published).
    let kinds: Vec<(String, String, String, String, Option<String>)> = db
        .read(|tx| {
            let mut st = tx
                .prepare(
                    "SELECT k.kind, k.searchable, k.id_pattern, \
                            (SELECT sql FROM sqlite_master WHERE type = 'view' AND name = k.searchable) \
                     FROM ref_kind k WHERE k.searchable IS NOT NULL ORDER BY k.kind",
                )
                .map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([], |r| {
                    let kind: String = r.get(0)?;
                    Ok((kind.clone(), r.get(1)?, format!("{kind}:"), r.get(2)?, r.get(3)?))
                })
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows)
        })
        .await?;
    // Core's kinds, from their views as the registry has them.
    let core: Vec<(String, String, String, String, Option<String>)> = db
        .read(|tx| {
            CORE_KINDS
                .iter()
                .map(|(kind, view, prefix, id)| {
                    let sql: Option<String> = tx
                        .query_row(
                            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
                            [view],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok((
                        kind.to_string(),
                        view.to_string(),
                        prefix.to_string(),
                        id.to_string(),
                        sql,
                    ))
                })
                .collect()
        })
        .await?;
    let core_kinds: std::collections::BTreeSet<String> =
        core.iter().map(|(kind, ..)| kind.clone()).collect();
    let kinds: Vec<_> = kinds.into_iter().chain(core).collect();
    let views: Vec<String> = kinds.iter().map(|(_, view, ..)| view.clone()).collect();
    let tables = crate::assets::tables_behind(db, &views).await?;
    let declared = kinds.iter().map(|(kind, ..)| kind.clone()).collect();
    let ready = kinds
        .into_iter()
        // A view the compiler hasn't published (yet, or any more) has
        // nothing to index; it registers once the registry lists it.
        .filter_map(|(kind, view, ref_prefix, id_pattern, sql)| {
            let tables = tables.get(&view)?.clone();
            let core = core_kinds.contains(&kind);
            Some((
                kind,
                SearchableKind {
                    core,
                    view,
                    sql: sql?,
                    ref_prefix,
                    id_pattern,
                    tables,
                },
            ))
        })
        .collect();
    Ok(Searchable { declared, ready })
}

impl Assets {
    /// Keep one search index per searchable plugin kind (see the module):
    /// new or changed ones (re)registered, gone ones stopped and their
    /// entries removed — also those of a kind that went while oxplow
    /// wasn't running (an `asset_state` row with no kind behind it). A
    /// kind still declared searchable whose view isn't published — at
    /// start, before the extension models compile again — keeps its
    /// entries: "not compiled yet" isn't "gone" (tsk852). Run at start and
    /// when `ref_kind` or the model registry changes.
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
            self.register(Arc::new(KindSearchIndex {
                db: self.db().clone(),
                asset: asset_name(kind),
                kind: kind.clone(),
                spec: spec.clone(),
            }));
            have.insert(kind.clone(), spec.clone());
        }
        // What was indexed for a kind that is no longer searchable goes —
        // found wherever it is left: its state, its failure, or entries
        // with neither (a restate that committed after a cleanup, or a
        // process that died between the two), core's kinds aside (tsk853).
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
                // Core's kinds are always declared; files aren't a model's.
                let gone = found.iter().filter(|kind| {
                    !keep.contains(kind)
                        && !CORE_KINDS.iter().any(|(k, ..)| k == kind)
                        && kind.as_str() != crate::indexer::KIND_FILE
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

    /// `acme`: pull requests collected as `pr` rows, a `prs` model that
    /// titles them, a `found` model that makes them searchable, and the
    /// kind `acme_pr` (`searchable` as given).
    fn acme(root: &Path, searchable: &str) {
        write(
            root,
            "oxplow/extensions/acme/extension.yaml",
            &format!(
                "manifest: 2
name: acme
sharing: private
intent: {{ purpose: PRs., origin: null, examples: [] }}
collectors:
  - id: prs
    runtime: starlark
    entry: prs.star
    entities:
      - {{ name: pr, key: n, columns: {{ n: text, title: text, body: text }} }}
models:
  - name: prs
    version: 1
    description: Pull requests.
    columns:
      - {{ name: ref, type: TEXT, doc: The ref. }}
      - {{ name: title, type: TEXT, doc: Its title. }}
  - name: found
    version: 1
    description: Pull requests, as search finds them.
    columns:
      - {{ name: ref, type: TEXT, doc: The ref. }}
      - {{ name: title, type: TEXT, doc: Its title. }}
      - {{ name: body, type: TEXT, doc: Its description. }}
pages:
  - {{ id: pr, title: Pull request, category: Work, lens: open }}
ref_kinds:
  - kind: acme_pr
    label: Pull request
    id: '^\\d+$'
    resolve: prs
    page: pr
{searchable}    icon: git-pull-request
"
            ),
        );
        write(
            root,
            "oxplow/extensions/acme/models/prs.sql",
            "SELECT CAST('acme_pr:' || n AS TEXT) AS ref, title FROM ref('pr')",
        );
        write(
            root,
            "oxplow/extensions/acme/models/found.sql",
            "SELECT CAST('acme_pr:' || n AS TEXT) AS ref, title, body FROM ref('pr')",
        );
        write(
            root,
            "oxplow/extensions/acme/lenses/open.yaml",
            "title: Open\nquery: \"SELECT 1 AS n\"\n",
        );
    }

    fn rows(root: &Path, rows: &str) {
        write(
            root,
            "oxplow/extensions/acme/prs.star",
            &format!("def transform(input):\n    return {{\"entities\": {{\"pr\": {rows}}}}}\n"),
        );
    }

    async fn collect(svc: &crate::Services) {
        let root = svc.layout.project_dir.clone();
        collector_runner::run_collector(
            &Collectors::of(svc, &root),
            "acme",
            "prs",
            collector_runner::RunTrigger::Manual,
            "human",
        )
        .await
        .unwrap();
    }

    /// Search for `query` until it finds `want` refs of `acme_pr` (the
    /// index follows its tables after a quiet moment).
    async fn found(svc: &crate::Services, query: &str, want: &[&str]) -> Vec<(String, String)> {
        let mut hits = Vec::new();
        for _ in 0..100 {
            hits = svc
                .search_store
                .search(query, None, &["acme_pr".to_string()], 10)
                .await
                .unwrap()
                .into_iter()
                .map(|h| (h.ref_id, h.title))
                .collect();
            let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
            if ids == want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        hits
    }

    /// P9.D3: a searchable kind's model rows are in the site-wide index
    /// under the kind, following the model as it changes; a row whose ref
    /// isn't one of the kind's is skipped; a kind that stops being
    /// searchable (or goes) takes its entries with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_searchable_kinds_rows_are_found_and_follow_the_model() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        acme(&root, "    searchable: found\n");
        rows(
            &root,
            r#"[{"n": "12", "title": "Widget frobnicator", "body": "Adds the frobnication of widgets."}, {"n": "x1", "title": "Widget impostor", "body": "Not a pull request's id."}]"#,
        );
        // The first sync publishes the `pr` entity its models read.
        collect(svc).await;
        svc.extension_models.sync().await.unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        let errors: Vec<String> = svc
            .listed_extensions(&root)
            .await
            .into_iter()
            .filter(|e| e.name == "acme")
            .flat_map(|e| e.errors)
            .collect();
        assert_eq!(errors, Vec::<String>::new());
        let listed = svc
            .sql
            .query_sql(
                "SELECT searchable FROM v_ref_kind WHERE kind = 'acme_pr'",
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
        crate::models_changed::spawn(
            svc.db.clone(),
            Arc::new(crate::models_changed::ModelWatermarks::default()),
            svc.events.clone(),
            assets.clone(),
            svc.event_pump.clone(),
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
                by_body[0].kind.as_str(),
                by_body[0].ref_id.as_str(),
                by_body[0].stream_id.clone()
            ),
            ("acme_pr", "12", None)
        );
        assert!(
            by_body[0].snippet.contains("frobnication"),
            "{:?}",
            by_body[0]
        );

        // The model's rows change: so does what is found.
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

        // Its model's SQL changes, its tables don't (tsk851): the index
        // follows the new SQL without waiting for a collector to write.
        write(
            &root,
            "oxplow/extensions/acme/models/found.sql",
            "SELECT CAST('acme_pr:' || n AS TEXT) AS ref, CAST(title || ' (open)' AS TEXT) AS title, body FROM ref('pr')",
        );
        svc.extension_models.sync().await.unwrap();
        assert_eq!(
            found(svc, "open", &["13"]).await,
            vec![("13".to_string(), "Gadget polish (open)".to_string())]
        );

        // No longer searchable: its entries leave.
        acme(&root, "");
        svc.vocabulary_service.sync().await.unwrap();
        assert_eq!(found(svc, "gadget", &[]).await, vec![]);
        let state = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM asset_state WHERE asset = 'search:acme_pr'",
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
    /// may have held the `ref_kind` or `model` write — so it resyncs the
    /// registry as if those changed, not only the assets' inputs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lagged_listener_resyncs_the_searchable_kinds() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        acme(&root, "    searchable: found\n");
        rows(
            &root,
            r#"[{"n": "12", "title": "Widget frobnicator", "body": "Frobs."}]"#,
        );
        collect(svc).await;
        svc.extension_models.sync().await.unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        // Nothing heard the `ref_kind` write: the listener missed it.
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(50));
        let mut lineage = crate::models_changed::Lineage::load(&svc.db).await.unwrap();
        crate::models_changed::react(
            &svc.db,
            &mut lineage,
            &assets,
            &svc.event_pump,
            &crate::models_changed::Changed::Everything,
        )
        .await;
        assert_eq!(
            found(svc, "widget", &["12"]).await,
            vec![("12".to_string(), "Widget frobnicator".to_string())]
        );
    }

    /// tsk852: at start, extension models are dropped and compiled again,
    /// so for a moment a searchable kind's view isn't there. That is "not
    /// compiled yet", not "gone": its entries stay until `ref_kind` stops
    /// saying it is searchable.
    #[tokio::test]
    async fn a_kind_whose_view_isnt_compiled_yet_keeps_its_entries() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.search_store
            .upsert("acme_pr", "12", None, "Widget frobnicator", "")
            .await
            .unwrap();
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO ref_kind (kind, extension, label, id_pattern, revisioned, wikilinks, searchable) \
                     VALUES ('acme_pr', 'acme', 'Pull request', '^\\d+$', 0, '[]', 'v_acme_found')",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms) \
                     VALUES ('search:acme_pr', '2026-01-01T00:00:00Z', 0, 1)",
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
            .map(|h| h.kind)
            .collect();
        assert_eq!(left, vec!["acme_pr"]);
    }

    /// tsk853: what a gone kind left is found from the index and the
    /// failures too, not only `asset_state`: a restate that committed
    /// after the cleanup (or a process that died between the two) left
    /// entries with no state row, and a kind whose recomputes only ever
    /// failed has a failure row and nothing else.
    #[tokio::test]
    async fn a_gone_kinds_orphaned_entries_and_failures_leave() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        for (kind, id) in [("acme_pr", "12"), ("wiki", "widgets"), ("task", "7")] {
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
            .map(|h| h.kind)
            .collect();
        left.sort();
        assert_eq!(left, vec!["task", "wiki"], "core's entries are untouched");
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
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.search_store
            .upsert("acme_pr", "12", None, "Widget frobnicator", "")
            .await
            .unwrap();
        svc.search_store
            .upsert("wiki", "widgets", None, "Widgets", "")
            .await
            .unwrap();
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms) \
                     VALUES ('search:acme_pr', '2026-01-01T00:00:00Z', 0, 1)",
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
            .map(|h| h.kind)
            .collect();
        assert_eq!(left, vec!["wiki"], "core's entries are untouched");
    }
}

/// Core's kinds (tsk864): tasks, comments, thread notes and wiki pages,
/// indexed from `v_search_<kind>` by the same assets as a plugin kind.
#[cfg(test)]
mod core_tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use oxplow_domain::stores::{TaskStore as _, ThreadStore as _};
    use oxplow_domain::{CommentTarget, StreamId, ThreadId};

    use crate::assets::Assets;
    use crate::Services;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Arc::new(Services::in_memory(dir.path()).expect("in-memory services"));
        (svc, dir)
    }

    /// The change loop, as boot runs it, over assets that settle fast.
    fn change_loop(svc: &Services) -> Assets {
        let assets = Assets::new(svc.db.clone(), Duration::from_millis(30));
        crate::models_changed::spawn(
            svc.db.clone(),
            Arc::new(crate::models_changed::ModelWatermarks::default()),
            svc.events.clone(),
            assets.clone(),
            svc.event_pump.clone(),
        );
        assets
    }

    /// Wait (up to 5 s) for `check` to hold.
    async fn eventually<F, Fut>(what: &str, mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        for _ in 0..200 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("never: {what}");
    }

    /// The index's `(ref_id, stream)` entries of `kind`.
    async fn entries(svc: &Services, kind: &'static str) -> Vec<(String, Option<String>)> {
        svc.db
            .read(move |c| {
                let mut s = c
                    .prepare(
                        "SELECT ref_id, stream_id FROM search_entry WHERE kind = ?1 ORDER BY ref_id",
                    )
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
            .any(|h| h.kind == kind)
    }

    async fn task(svc: &Services, thread: ThreadId, title: &str) -> oxplow_domain::Task {
        svc.tasks
            .create(
                Some(thread),
                crate::CreateTaskInput {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
    }

    /// tsk864: a task is indexed from `v_search_task` — written, edited,
    /// found by its id, gone when deleted — its kind restated as a whole.
    #[tokio::test]
    async fn a_task_edit_restates_its_kind() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let t = task(&svc, thread.id, "Quux the sprocket").await;
        eventually("the new task is found", || {
            found(&svc, "sprocket", Some(stream.id), "task")
        })
        .await;
        let id = t.id.to_string();
        assert!(
            found(&svc, &id, Some(stream.id), "task").await,
            "its id finds it"
        );

        let mut edited = t.clone();
        edited.title = "Quux the flange".into();
        svc.task_store.update(&edited).await.unwrap();
        eventually("the edit is found", || {
            found(&svc, "flange", Some(stream.id), "task")
        })
        .await;
        assert!(!found(&svc, "sprocket", Some(stream.id), "task").await);

        svc.task_store.soft_delete(t.id).await.unwrap();
        eventually("the deleted task leaves", || async {
            entries(&svc, "task").await.is_empty()
        })
        .await;
    }

    /// tsk864: a thread's tasks go with it — what the old upsert-only
    /// backfill never removed.
    #[tokio::test]
    async fn a_deleted_threads_tasks_leave_the_index() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let kept = crate::test_fixtures::new_thread(&svc, stream.id, "kept").await;
        let gone = crate::test_fixtures::new_thread(&svc, stream.id, "gone").await;
        change_loop(&svc);
        let stays = task(&svc, kept.id, "Stays put").await;
        task(&svc, gone.id, "Goes with its thread").await;
        eventually("both are indexed", || async {
            entries(&svc, "task").await.len() == 2
        })
        .await;
        svc.thread_store.delete(&gone.id).await.unwrap();
        eventually("only the kept thread's task is left", || async {
            entries(&svc, "task").await == vec![(stays.id.to_string(), Some(stream.id.to_string()))]
        })
        .await;
    }

    /// tsk922: a core feed model is published like any other, so its `ref`
    /// is a canonical ref — the one a hit opens — and its rows still index
    /// under the search kind.
    #[tokio::test]
    async fn core_feed_refs_are_canonical() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let t = task(&svc, thread.id, "Quux the sprocket").await;
        eventually("indexed", || async {
            entries(&svc, "task").await.len() == 1
        })
        .await;
        let refs: Vec<String> = svc
            .db
            .read(|c| {
                let mut s = c
                    .prepare("SELECT ref FROM v_search_task")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<String>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(refs, vec![format!("work_item:oxplow:{}", t.id)]);
        let kinds = oxplow_domain::refs::kind::core_kinds();
        for r in &refs {
            oxplow_domain::refs::validate_ref(&kinds, r).unwrap();
        }
        assert_eq!(
            entries(&svc, "task").await,
            vec![(t.id.to_string(), Some(stream.id.to_string()))]
        );
    }

    /// tsk921: archived work leaves search as its files do — an archived
    /// thread's tasks, then an archived stream's — and the backlog stays.
    #[tokio::test]
    async fn archived_threads_and_streams_leave_search() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let kept = crate::test_fixtures::new_thread(&svc, stream.id, "kept").await;
        let shelved = crate::test_fixtures::new_thread(&svc, stream.id, "shelved").await;
        change_loop(&svc);
        let stays = task(&svc, kept.id, "Stays put").await;
        task(&svc, shelved.id, "Shelved with its thread").await;
        let backlog = svc
            .tasks
            .create(
                None,
                crate::CreateTaskInput {
                    title: "On the backlog".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        eventually("all three are indexed", || async {
            entries(&svc, "task").await.len() == 3
        })
        .await;
        oxplow_domain::stores::ThreadStore::archive(&*svc.thread_store, &shelved.id)
            .await
            .unwrap();
        eventually("the archived thread's task leaves", || async {
            entries(&svc, "task").await
                == vec![
                    (stays.id.to_string(), Some(stream.id.to_string())),
                    (backlog.id.to_string(), None),
                ]
        })
        .await;
        oxplow_domain::stores::StreamStore::archive(&*svc.stream_store, &stream.id)
            .await
            .unwrap();
        eventually(
            "the archived stream's tasks leave; the backlog stays",
            || async { entries(&svc, "task").await == vec![(backlog.id.to_string(), None)] },
        )
        .await;
    }

    /// A task moved to the backlog is indexed there and nowhere else
    /// (tsk508): its entry's stream follows its thread.
    #[tokio::test]
    async fn a_moved_task_is_indexed_only_where_it_now_lives() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        change_loop(&svc);
        let t = task(&svc, thread.id, "Quux the sprocket").await;
        let id = t.id.to_string();
        eventually("in its thread's stream", || async {
            entries(&svc, "task").await == vec![(id.clone(), Some(stream.id.to_string()))]
        })
        .await;
        svc.task_store.move_task(t.id, None).await.unwrap();
        eventually("in the backlog", || async {
            entries(&svc, "task").await == vec![(id.clone(), None)]
        })
        .await;
    }

    /// tsk864: a restart re-registers the kinds and recomputes them — and,
    /// their rows unchanged, writes nothing: the index entries are the ones
    /// already there.
    #[tokio::test]
    async fn boot_rebuilds_nothing_when_assets_are_fresh() {
        let (svc, _dir) = services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        task(&svc, thread.id, "Quux the sprocket").await;
        change_loop(&svc);
        eventually("indexed", || {
            found(&svc, "sprocket", Some(stream.id), "task")
        })
        .await;
        let computed = |svc: Arc<Services>| async move {
            svc.db
                .read(|c| {
                    c.query_row(
                        "SELECT computed_at FROM asset_state WHERE asset = 'search:task'",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        let first = computed(svc.clone()).await;
        // tsk897: hear every commit from here on — a rewrite of the index
        // touches `search_entry` (rowids alone can't tell: a delete and
        // re-insert hands the same ones back).
        let mut commits = svc.db.subscribe_changes();
        // A restart: a new change loop registers every kind again, and each
        // builds once.
        tokio::time::sleep(Duration::from_millis(20)).await;
        change_loop(&svc);
        eventually("the restart recomputed the task kind", || async {
            computed(svc.clone()).await != first
        })
        .await;
        let mut touched = Vec::new();
        while let Ok(changed) = commits.try_recv() {
            touched.extend(changed.tables.iter().cloned());
        }
        assert!(
            !touched.iter().any(|t| t == "search_entry"),
            "the restart rewrote the index: {touched:?}"
        );
    }

    /// Run `name` as a person; its result.
    async fn run(svc: &Services, name: &str, input: serde_json::Value) -> serde_json::Value {
        svc.commands
            .run(&oxplow_domain::Actor::Human, name, input, true)
            .await
            .unwrap()
            .result
    }

    /// Wiki pages: written by command or by hand, gone when deleted.
    #[tokio::test]
    async fn wiki_pages_are_indexed() {
        let (svc, dir) = services().await;
        svc.streams.ensure_primary().await.unwrap();
        change_loop(&svc);
        run(
            &svc,
            crate::knowledge::WRITE_PAGE,
            serde_json::json!({ "slug": "gears", "body": "# Gears\n\nThe sprocket turns.\n" }),
        )
        .await;
        eventually("the page is found", || {
            found(&svc, "sprocket", None, "wiki")
        })
        .await;
        std::fs::write(
            crate::knowledge::page_path(dir.path(), "gears"),
            "# Gears\n\nThe flange holds.\n",
        )
        .unwrap();
        crate::wiki_pages::sync_page(&svc.db, &svc.vocabulary, dir.path(), "gears")
            .await
            .unwrap();
        eventually("a hand edit is found", || {
            found(&svc, "flange", None, "wiki")
        })
        .await;
        run(
            &svc,
            crate::knowledge::DELETE_PAGE,
            serde_json::json!({ "slug": "gears" }),
        )
        .await;
        eventually("the deleted page leaves", || async {
            entries(&svc, "wiki").await.is_empty()
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
            crate::commands::note::ADD,
            serde_json::json!({ "thread": oxplow_domain::refs::build::thread_ref(thread.id), "body": "a note about the gadget" }),
        )
        .await;
        let note: oxplow_domain::NoteId =
            serde_json::from_value(out["note"]["id"].clone()).unwrap();
        eventually("the note is found", || {
            found(&svc, "gadget", Some(stream.id), "note")
        })
        .await;
        let vocabulary = svc.vocabulary.clone();
        svc.db
            .transaction(move |tx| {
                if let Some(e) = oxplow_db::task_satellite::delete_note_tx(tx, note)? {
                    oxplow_db::event_log_store::append_tx(tx, &vocabulary.current(), &e)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        eventually("the deleted note leaves", || async {
            entries(&svc, "note").await.is_empty()
        })
        .await;

        let out = run(
            &svc,
            crate::commands::comment::ADD,
            serde_json::json!({
                "stream": oxplow_domain::refs::build::stream_ref(stream.id),
                "thread": oxplow_domain::refs::build::thread_ref(thread.id),
                "target": CommentTarget { kind: "file".into(), id: "src/lib.rs".into() },
                "quote": "quoted text", "body": "this mentions the doohickey",
            }),
        )
        .await;
        let comment = out["comment"]["id"].clone();
        eventually("the comment is found", || {
            found(&svc, "doohickey", Some(stream.id), "comment")
        })
        .await;
        assert!(
            found(&svc, "quoted", Some(stream.id), "comment").await,
            "by its quote too"
        );
        run(
            &svc,
            crate::commands::comment::REPLY,
            serde_json::json!({ "comment": comment, "body": "and the thingamajig" }),
        )
        .await;
        eventually("a reply is found", || {
            found(&svc, "thingamajig", Some(stream.id), "comment")
        })
        .await;
        run(
            &svc,
            crate::commands::comment::DELETE,
            serde_json::json!({ "comment": comment }),
        )
        .await;
        eventually("the deleted comment leaves", || async {
            entries(&svc, "comment").await.is_empty()
        })
        .await;
    }
}
