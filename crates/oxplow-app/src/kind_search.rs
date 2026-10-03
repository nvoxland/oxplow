//! Search over plugin ref kinds (P9.D3, `.context/refs.md` "Searchable
//! kinds"): a kind declared `searchable: <model>` has that model's rows
//! (`ref`, `title`, `body`) in the site-wide index (`search_fts`) under
//! its kind, so the launcher finds `[[pr:12]]` by its title.
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
//! Bounded: at most [`MAX_ROWS`] rows a kind, each body cut at
//! [`MAX_BODY`] bytes. A row whose `ref` isn't one of the kind's
//! (`<kind>:<id>`, the id matching its pattern) is skipped.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_db::Database;
use oxplow_domain::DomainError;

use crate::assets::{Assets, Materializer, Recomputed};

/// The most rows of one kind the index holds.
pub const MAX_ROWS: usize = 20_000;
/// The most bytes of one row's body it indexes.
pub const MAX_BODY: usize = 16 * 1024;

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
    id_pattern: String,
    /// The tables behind the view.
    tables: Vec<String>,
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
        let (kind, view) = (self.kind.clone(), self.spec.view.clone());
        let id = regex::Regex::new(&self.spec.id_pattern)
            .map_err(|e| DomainError::Invalid(format!("ref kind `{kind}`'s id pattern: {e}")))?;
        let indexed = self
            .db
            .transaction(move |tx| {
                let sql = format!(
                    "SELECT CAST(ref AS TEXT), CAST(title AS TEXT), CAST(body AS TEXT) \
                     FROM \"{}\" LIMIT {}",
                    view.replace('"', "\"\""),
                    MAX_ROWS + 1
                );
                let mut st = tx.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
                let rows: Vec<(Option<String>, Option<String>, Option<String>)> = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)?;
                if rows.len() > MAX_ROWS {
                    tracing::warn!(%kind, %view, "more than {MAX_ROWS} rows; the rest aren't searchable");
                }
                let prefix = format!("{kind}:");
                let mut entries: BTreeMap<String, (String, String)> = BTreeMap::new();
                let mut skipped = 0usize;
                for (r, title, body) in rows.into_iter().take(MAX_ROWS) {
                    // One of the kind's refs, or it isn't something a hit
                    // could open.
                    let Some(ref_id) = r
                        .as_deref()
                        .and_then(|r| r.strip_prefix(&prefix))
                        .filter(|id_text| id.is_match(id_text))
                    else {
                        skipped += 1;
                        continue;
                    };
                    let mut body = body.unwrap_or_default();
                    if body.len() > MAX_BODY {
                        let mut end = MAX_BODY;
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        body.truncate(end);
                    }
                    entries.insert(ref_id.to_string(), (title.unwrap_or_default(), body));
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

/// The searchable kinds `ref_kind` lists, each with the tables behind its
/// view.
async fn searchable_kinds(db: &Database) -> Result<BTreeMap<String, SearchableKind>, DomainError> {
    // Each kind with its view's compiled SQL (none while the view isn't
    // published).
    let kinds: Vec<(String, String, String, Option<String>)> = db
        .read(|tx| {
            let mut st = tx
                .prepare(
                    "SELECT k.kind, k.searchable, k.id_pattern, \
                            (SELECT sql FROM sqlite_master WHERE type = 'view' AND name = k.searchable) \
                     FROM ref_kind k WHERE k.searchable IS NOT NULL ORDER BY k.kind",
                )
                .map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows)
        })
        .await?;
    let views: Vec<String> = kinds.iter().map(|(_, view, _, _)| view.clone()).collect();
    let tables = crate::assets::tables_behind(db, &views).await?;
    Ok(kinds
        .into_iter()
        // A view the compiler hasn't published (yet, or any more) has
        // nothing to index; it registers once the registry lists it.
        .filter_map(|(kind, view, id_pattern, sql)| {
            let tables = tables.get(&view)?.clone();
            Some((
                kind,
                SearchableKind {
                    view,
                    sql: sql?,
                    id_pattern,
                    tables,
                },
            ))
        })
        .collect())
}

impl Assets {
    /// Keep one search index per searchable plugin kind (see the module):
    /// new or changed ones (re)registered, gone ones stopped and their
    /// entries removed — also those of a kind that went while oxplow
    /// wasn't running (an `asset_state` row with no kind behind it). Run
    /// at start and when `ref_kind` or the model registry changes.
    pub async fn sync_search_kinds(&self) -> Result<(), DomainError> {
        let wanted = searchable_kinds(self.db()).await?;
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
        // What was indexed for a kind that is no longer searchable goes.
        let keep: Vec<String> = wanted.keys().map(|k| asset_name(k)).collect();
        self.db()
            .transaction(move |tx| {
                let mut st = tx
                    .prepare("SELECT asset FROM asset_state WHERE asset LIKE 'search:%'")
                    .map_err(oxplow_db::map_sql_err)?;
                let recorded: Vec<String> = st
                    .query_map([], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)?;
                for asset in recorded.iter().filter(|a| !keep.contains(a)) {
                    let kind = asset.trim_start_matches("search:");
                    oxplow_db::search_store::restate_kind_tx(tx, kind, &BTreeMap::new())?;
                    for table in ["asset_state", "asset_failure"] {
                        tx.execute(&format!("DELETE FROM {table} WHERE asset = ?1"), [asset])
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
