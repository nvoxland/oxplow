//! SQLite-backed unified cross-page reference graph.
//!
//! Each row is one directed edge `(source) --ref_type--> (target)`, each
//! end a URI (`.context/resources.md`). Per-subsystem writers own their
//! sources' rows (the wiki sync owns every wiki page's, the task save
//! path every work item's, …). The standard write pattern is
//! [`SqlitePageRefStore::replace_source`]: delete every row whose source
//! matches, then re-insert the new edge set in one transaction. The
//! reader looks up `target_uri` for backlinks or `source_uri` for
//! outbound — both indexed.
//!
//! Each end's resource (`oxplow/file`) is stored beside its URI so a
//! resource-filtered lookup ("all backlinks from a wiki page") needs no
//! parse of the URI.

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::DomainError;

use crate::database::Database;

/// One directed edge in the page graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PageRefEdge {
    /// The URI that refers.
    pub source: String,
    /// The URI it refers to.
    pub target: String,
    pub ref_type: String,
    /// Optional JSON blob with edge-specific extras (line anchor,
    /// version pin, link sub-type, …). Opaque to the store.
    pub source_extra: Option<String>,
    /// Version data for edges pointing at a file-like target. Set
    /// by the writer via [`PageRefEdge::with_version`]; the store
    /// persists it verbatim. See V20 for column semantics.
    pub local_snapshot_id: Option<i64>,
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
}

impl PageRefEdge {
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        ref_type: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            ref_type: ref_type.into(),
            source_extra: None,
            local_snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
        }
    }

    pub fn with_extra(mut self, extra: impl Into<String>) -> Self {
        self.source_extra = Some(extra.into());
        self
    }

    /// Stamp a file-version pin on this edge. Callers only invoke
    /// this for file-like targets (file / directory / git_commit);
    /// the store doesn't enforce that.
    pub fn with_version(
        mut self,
        local_snapshot_id: i64,
        closest_vcs_rev: Option<String>,
        vcs_rev_exact: bool,
    ) -> Self {
        self.local_snapshot_id = Some(local_snapshot_id);
        self.closest_vcs_rev = closest_vcs_rev;
        self.vcs_rev_exact = vcs_rev_exact;
        self
    }
}

/// The resource a stored end names (`oxplow/file`), for its column.
fn resource(uri: &str) -> String {
    oxplow_domain::uri::build::resource_of(uri).unwrap_or_default()
}

#[derive(Clone)]
pub struct SqlitePageRefStore {
    db: Database,
}

/// One source's edges for [`SqlitePageRefStore::replace_sources`]: the
/// whole slice it owns (`ref_types: None`), or only the rows of the given
/// `ref_types`.
#[derive(Debug, Clone)]
pub struct SourceSlice {
    pub source: String,
    pub ref_types: Option<Vec<String>>,
    pub edges: Vec<PageRefEdge>,
}

const COLUMNS: &str = "source_uri, target_uri, ref_type, source_extra, local_snapshot_id, \
                       closest_vcs_rev, vcs_rev_exact";

impl SqlitePageRefStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace every edge owned by `source` with `edges`. Atomic.
    /// Idempotent: passing the same `edges` twice leaves the table
    /// unchanged. Edges in `edges` whose source isn't `source` are
    /// silently skipped — callers shouldn't construct mixed batches but
    /// the store stays defensive.
    pub async fn replace_source(
        &self,
        source: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        let source = source.to_string();
        self.db
            .transaction(move |tx| {
                replace_source_tx(tx, &source, edges.clone())?;
                Ok(())
            })
            .await
    }

    /// [`replace_source`] / [`replace_source_for_ref_types`] for many
    /// sources in one transaction — one commit, one change event, where a
    /// call per source is a commit (and an event the UI hears) each. For
    /// restating many rows at once (the boot repair).
    pub async fn replace_sources(&self, slices: Vec<SourceSlice>) -> Result<(), DomainError> {
        if slices.is_empty() {
            return Ok(());
        }
        self.db
            .transaction(move |tx| {
                for s in &slices {
                    match &s.ref_types {
                        None => replace_source_tx(tx, &s.source, s.edges.clone())?,
                        Some(types) if types.is_empty() => {}
                        Some(types) => {
                            replace_source_for_ref_types_tx(tx, &s.source, types, s.edges.clone())?
                        }
                    }
                }
                Ok(())
            })
            .await
    }

    /// Diff-merge: preserve version metadata on edges that already
    /// exist (matched on the PK), insert new edges with the caller's
    /// version data, delete edges in storage but not in the new
    /// set. Atomic. Used by the wiki sync so editing unrelated prose
    /// doesn't re-stamp every file ref's `local_snapshot_id`.
    ///
    /// Semantics:
    /// - Edge in `existing ∩ new` → row kept with its OLD
    ///   `local_snapshot_id` / `closest_vcs_rev` /
    ///   `vcs_rev_exact`. `source_extra` is updated to the new
    ///   value (line anchors, label overrides may legitimately
    ///   change without re-verifying the target).
    /// - Edge in `new \ existing` → INSERT with the new edge's
    ///   version data (caller stamps it via
    ///   `page_ref_projections::stamp_file_versions`).
    /// - Edge in `existing \ new` → DELETE.
    pub async fn merge_source(
        &self,
        source: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        let source = source.to_string();
        self.db
            .transaction(move |tx| {
                merge_source_tx(tx, &source, edges.clone())?;
                Ok(())
            })
            .await
    }

    /// Insert or replace a single edge, stamping the caller-supplied
    /// version data. Unlike `merge_source`/`replace_source`, this
    /// touches exactly one `(source, target, ref_type)` row and never
    /// prunes. Used to **materialize a verification edge** — e.g. a
    /// wiki page the agent verified against a file it cites only via
    /// the file's containing directory, which the body parser would
    /// not emit. The wiki sync preserves such edges as long as their
    /// path stays under a cited directory ref.
    pub async fn upsert_edge(&self, edge: PageRefEdge) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| upsert_edge_tx(tx, &edge))
            .await
    }

    /// Like [`replace_source`] but restricted to a named slice
    /// identified by `ref_types`: only rows whose `ref_type` matches
    /// one of the supplied values are deleted, then `edges` is
    /// inserted. Used when a single source has multiple writers
    /// contributing different `ref_type`s — e.g. a work item gets
    /// body-mention edges from the task upsert, link edges from the
    /// link store, and touched-file edges from the effort store. Each
    /// writer passes only its own ref_types so the others' rows survive.
    pub async fn replace_source_for_ref_types(
        &self,
        source: &str,
        ref_types: Vec<String>,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        if ref_types.is_empty() {
            return Ok(());
        }
        let source = source.to_string();
        self.db
            .transaction(move |tx| {
                replace_source_for_ref_types_tx(tx, &source, &ref_types, edges.clone())
            })
            .await
    }

    /// Edges that point AT `target` — i.e. the classic backlinks list.
    /// Ordered by source for stable rendering; callers can re-sort.
    pub async fn list_backlinks(
        &self,
        target: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let target = target.to_string();
        let limit = limit.unwrap_or(500);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {COLUMNS} FROM page_ref WHERE target_uri = ?1
                     ORDER BY source_uri, ref_type LIMIT ?2"
                ))?;
                let rows = stmt.query_map(params![target, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Edges emitted FROM `source` — the page's own outbound list.
    pub async fn list_outbound(
        &self,
        source: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let source = source.to_string();
        let limit = limit.unwrap_or(500);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {COLUMNS} FROM page_ref WHERE source_uri = ?1
                     ORDER BY target_uri, ref_type LIMIT ?2"
                ))?;
                let rows = stmt.query_map(params![source, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageRefEdge> {
    let vcs_rev_exact: i64 = row.get(6)?;
    Ok(PageRefEdge {
        source: row.get(0)?,
        target: row.get(1)?,
        ref_type: row.get(2)?,
        source_extra: row.get(3)?,
        local_snapshot_id: row.get(4)?,
        closest_vcs_rev: row.get(5)?,
        vcs_rev_exact: vcs_rev_exact != 0,
    })
}

#[async_trait]
pub trait PageRefStore: Send + Sync {
    async fn replace_source(
        &self,
        source: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError>;
    async fn list_backlinks(
        &self,
        target: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
    async fn list_outbound(
        &self,
        source: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
}

#[async_trait]
impl PageRefStore for SqlitePageRefStore {
    async fn replace_source(
        &self,
        source: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        SqlitePageRefStore::replace_source(self, source, edges).await
    }
    async fn list_backlinks(
        &self,
        target: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_backlinks(self, target, limit).await
    }
    async fn list_outbound(
        &self,
        source: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_outbound(self, source, limit).await
    }
}

/// Write one edge (`INSERT OR <conflict>`), its resources from its URIs.
fn insert(
    conn: &rusqlite::Connection,
    conflict: &str,
    edge: &PageRefEdge,
    version: (Option<i64>, Option<String>, bool),
) -> Result<(), DomainError> {
    conn.execute(
        &format!(
            "INSERT OR {conflict} INTO page_ref
               (source_resource, source_uri, target_resource, target_uri, ref_type,
                source_extra, local_snapshot_id, closest_vcs_rev, vcs_rev_exact)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        ),
        params![
            resource(&edge.source),
            edge.source,
            resource(&edge.target),
            edge.target,
            edge.ref_type,
            edge.source_extra,
            version.0,
            version.1,
            if version.2 { 1 } else { 0 },
        ],
    )
    .map_err(crate::database::map_sql_err)?;
    Ok(())
}

/// Sync core of [`SqlitePageRefStore::merge_source`] (its semantics):
/// edges already stored keep their version pins, new ones take theirs,
/// and stored edges not in `edges` go.
pub fn merge_source_tx(
    conn: &rusqlite::Connection,
    source: &str,
    edges: Vec<PageRefEdge>,
) -> Result<(), DomainError> {
    // The unique key for an edge within a source is (target, ref_type).
    type Key = (String, String);
    type Existing = std::collections::HashMap<Key, (Option<i64>, Option<String>, bool)>;
    let existing: Existing = {
        let mut stmt = conn
            .prepare(
                "SELECT target_uri, ref_type, local_snapshot_id, closest_vcs_rev, vcs_rev_exact
                 FROM page_ref WHERE source_uri = ?1",
            )
            .map_err(crate::database::map_sql_err)?;
        let rows = stmt
            .query_map(params![source], |r| {
                let target: String = r.get(0)?;
                let rt: String = r.get(1)?;
                let local: Option<i64> = r.get(2)?;
                let git: Option<String> = r.get(3)?;
                let exact: i64 = r.get(4)?;
                Ok(((target, rt), (local, git, exact != 0)))
            })
            .map_err(crate::database::map_sql_err)?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row.map_err(crate::database::map_sql_err)?;
            map.insert(k, v);
        }
        map
    };
    let edges: Vec<PageRefEdge> = edges.into_iter().filter(|e| e.source == source).collect();
    let new_keys: std::collections::HashSet<Key> = edges
        .iter()
        .map(|e| (e.target.clone(), e.ref_type.clone()))
        .collect();
    // Upsert each new edge. Match on PK; if existing, preserve version
    // data; otherwise stamp with the caller-supplied version.
    for edge in &edges {
        let key = (edge.target.clone(), edge.ref_type.clone());
        let version = match existing.get(&key) {
            Some(prev) => prev.clone(),
            None => (
                edge.local_snapshot_id,
                edge.closest_vcs_rev.clone(),
                edge.vcs_rev_exact,
            ),
        };
        insert(conn, "REPLACE", edge, version)?;
    }
    // Delete edges that existed but aren't in the new set.
    for key in existing.keys().filter(|k| !new_keys.contains(*k)) {
        conn.execute(
            "DELETE FROM page_ref WHERE source_uri = ?1 AND target_uri = ?2 AND ref_type = ?3",
            params![source, &key.0, &key.1],
        )
        .map_err(crate::database::map_sql_err)?;
    }
    Ok(())
}

/// Insert or replace one edge with its own version pin — the sync core
/// of [`SqlitePageRefStore::upsert_edge`].
pub fn upsert_edge_tx(conn: &rusqlite::Connection, edge: &PageRefEdge) -> Result<(), DomainError> {
    insert(
        conn,
        "REPLACE",
        edge,
        (
            edge.local_snapshot_id,
            edge.closest_vcs_rev.clone(),
            edge.vcs_rev_exact,
        ),
    )
}

/// Sync core of [`SqlitePageRefStore::replace_source`]: every edge owned
/// by `source` becomes `edges`.
pub fn replace_source_tx(
    conn: &rusqlite::Connection,
    source: &str,
    edges: Vec<PageRefEdge>,
) -> Result<(), DomainError> {
    conn.execute(
        "DELETE FROM page_ref WHERE source_uri = ?1",
        params![source],
    )
    .map_err(crate::database::map_sql_err)?;
    let ref_types: Vec<String> = edges
        .iter()
        .map(|e| e.ref_type.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    replace_source_for_ref_types_tx(conn, source, &ref_types, edges)
}

/// Insert the edges of sources that have none yet (a row just inserted, its
/// id new): [`replace_source_tx`] without its two deletes, and one prepared
/// statement for all. A finished scan files one source per finding, in the
/// thousands.
pub fn insert_new_sources_tx(
    conn: &rusqlite::Connection,
    edges: &[PageRefEdge],
) -> Result<(), DomainError> {
    let mut stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO page_ref
               (source_resource, source_uri, target_resource, target_uri, ref_type,
                source_extra, local_snapshot_id, closest_vcs_rev, vcs_rev_exact)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(crate::database::map_sql_err)?;
    for edge in edges {
        stmt.execute(params![
            resource(&edge.source),
            edge.source,
            resource(&edge.target),
            edge.target,
            edge.ref_type,
            edge.source_extra,
            edge.local_snapshot_id,
            edge.closest_vcs_rev,
            if edge.vcs_rev_exact { 1 } else { 0 },
        ])
        .map_err(crate::database::map_sql_err)?;
    }
    Ok(())
}

/// Sync core of [`SqlitePageRefStore::replace_source_for_ref_types`]:
/// delete the source's rows whose `ref_type` is in `ref_types`, then
/// insert `edges` (only those matching the source and one of the
/// ref_types). Composes inside a `Database::transaction` closure — the
/// event pump's consumers project through it.
pub fn replace_source_for_ref_types_tx(
    conn: &rusqlite::Connection,
    source: &str,
    ref_types: &[String],
    edges: Vec<PageRefEdge>,
) -> Result<(), DomainError> {
    if ref_types.is_empty() {
        return Ok(());
    }
    let placeholders: Vec<String> = (2..2 + ref_types.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "DELETE FROM page_ref WHERE source_uri = ?1 AND ref_type IN ({})",
        placeholders.join(",")
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + ref_types.len());
    params_vec.push(&source);
    for rt in ref_types {
        params_vec.push(rt);
    }
    conn.execute(&sql, &params_vec[..])
        .map_err(crate::database::map_sql_err)?;
    for edge in edges {
        if edge.source != source || !ref_types.contains(&edge.ref_type) {
            continue;
        }
        insert(
            conn,
            "IGNORE",
            &edge,
            (
                edge.local_snapshot_id,
                edge.closest_vcs_rev.clone(),
                edge.vcs_rev_exact,
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxplow_domain::uri::build::{file_uri, wiki_uri, work_item_uri};

    fn w(slug: &str) -> String {
        wiki_uri(slug)
    }

    fn f(path: &str) -> String {
        file_uri(path)
    }

    fn edge(source: &str, target: &str, ref_type: &str) -> PageRefEdge {
        PageRefEdge::new(source, target, ref_type)
    }

    #[tokio::test]
    async fn round_trip_single_edge() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(&w("architecture"), &f("src/app.rs"), "wiki_file_ref");
        store
            .replace_source(&w("architecture"), vec![e.clone()])
            .await
            .unwrap();

        let inbound = store.list_backlinks(&f("src/app.rs"), None).await.unwrap();
        assert_eq!(inbound, vec![e.clone()]);

        let outbound = store.list_outbound(&w("architecture"), None).await.unwrap();
        assert_eq!(outbound, vec![e]);
    }

    #[tokio::test]
    async fn upsert_edge_inserts_then_overwrites_version() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(&w("intro"), &f("crates/x/src/lib.rs"), "wiki_file_ref").with_version(
            100,
            Some("aaaa".into()),
            true,
        );
        store.upsert_edge(e).await.unwrap();
        let out = store.list_outbound(&w("intro"), None).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].local_snapshot_id, Some(100));

        // Re-verifying overwrites the pin (unlike merge_source, which
        // preserves) — an explicit verification advances the pin.
        let e2 = edge(&w("intro"), &f("crates/x/src/lib.rs"), "wiki_file_ref").with_version(
            200,
            Some("bbbb".into()),
            false,
        );
        store.upsert_edge(e2).await.unwrap();
        let out2 = store.list_outbound(&w("intro"), None).await.unwrap();
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].local_snapshot_id, Some(200));
        assert_eq!(out2[0].closest_vcs_rev.as_deref(), Some("bbbb"));
        assert!(!out2[0].vcs_rev_exact);
    }

    #[tokio::test]
    async fn merge_source_preserves_existing_version() {
        // The first write stamps an edge at snapshot 100. A second
        // merge with the same edge (but a different version stamp)
        // must keep the original snapshot id and exact flag.
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e1 = edge(&w("intro"), &f("a.rs"), "wiki_file_ref").with_version(
            100,
            Some("aaaa".into()),
            true,
        );
        store.merge_source(&w("intro"), vec![e1]).await.unwrap();
        let e2 = edge(&w("intro"), &f("a.rs"), "wiki_file_ref").with_version(
            200,
            Some("bbbb".into()),
            false,
        );
        store.merge_source(&w("intro"), vec![e2]).await.unwrap();
        let out = store.list_outbound(&w("intro"), None).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].local_snapshot_id, Some(100));
        assert_eq!(out[0].closest_vcs_rev.as_deref(), Some("aaaa"));
        assert!(out[0].vcs_rev_exact);
    }

    #[tokio::test]
    async fn merge_source_stamps_new_edges_and_deletes_missing() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let a = edge(&w("intro"), &f("a.rs"), "wiki_file_ref").with_version(100, None, false);
        let b = edge(&w("intro"), &f("b.rs"), "wiki_file_ref").with_version(100, None, false);
        store.merge_source(&w("intro"), vec![a, b]).await.unwrap();
        // Second merge: drop b.rs, add c.rs. a.rs preserved; b
        // deleted; c.rs newly stamped at snapshot 200.
        let a2 = edge(&w("intro"), &f("a.rs"), "wiki_file_ref").with_version(200, None, false);
        let c = edge(&w("intro"), &f("c.rs"), "wiki_file_ref").with_version(200, None, false);
        store.merge_source(&w("intro"), vec![a2, c]).await.unwrap();
        let out = store.list_outbound(&w("intro"), None).await.unwrap();
        assert_eq!(out.len(), 2);
        let by_target: std::collections::HashMap<_, _> = out
            .iter()
            .map(|e| (e.target.clone(), e.local_snapshot_id))
            .collect();
        assert_eq!(by_target.get(&f("a.rs")), Some(&Some(100)));
        assert_eq!(by_target.get(&f("c.rs")), Some(&Some(200)));
        assert!(!by_target.contains_key(&f("b.rs")));
    }

    #[tokio::test]
    async fn replace_source_is_idempotent() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let edges = vec![
            edge(&w("a"), &f("x.rs"), "wiki_file_ref"),
            edge(
                &w("a"),
                &work_item_uri("oxplow", "wi-1"),
                "work_item_mention",
            ),
        ];
        store.replace_source(&w("a"), edges.clone()).await.unwrap();
        store.replace_source(&w("a"), edges.clone()).await.unwrap();
        let out = store.list_outbound(&w("a"), None).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn replace_source_drops_removed_edges() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(
                &w("a"),
                vec![
                    edge(&w("a"), &f("x.rs"), "wiki_file_ref"),
                    edge(&w("a"), &f("y.rs"), "wiki_file_ref"),
                ],
            )
            .await
            .unwrap();
        // Re-save with only y.rs — x.rs must vanish.
        store
            .replace_source(&w("a"), vec![edge(&w("a"), &f("y.rs"), "wiki_file_ref")])
            .await
            .unwrap();
        let inbound_x = store.list_backlinks(&f("x.rs"), None).await.unwrap();
        let inbound_y = store.list_backlinks(&f("y.rs"), None).await.unwrap();
        assert!(inbound_x.is_empty());
        assert_eq!(inbound_y.len(), 1);
    }

    #[tokio::test]
    async fn replace_source_doesnt_touch_other_sources() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(&w("a"), vec![edge(&w("a"), &f("x.rs"), "wiki_file_ref")])
            .await
            .unwrap();
        store
            .replace_source(&w("b"), vec![edge(&w("b"), &f("x.rs"), "wiki_file_ref")])
            .await
            .unwrap();
        // Now replace `a` with empty — only `a`'s edges go.
        store.replace_source(&w("a"), vec![]).await.unwrap();
        let inbound = store.list_backlinks(&f("x.rs"), None).await.unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].source, w("b"));
    }

    #[tokio::test]
    async fn extra_payload_round_trips() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(&w("a"), &f("x.rs"), "wiki_file_ref")
            .with_extra(r#"{"line":42,"version":"@HEAD"}"#);
        store
            .replace_source(&w("a"), vec![e.clone()])
            .await
            .unwrap();
        let got = store.list_outbound(&w("a"), None).await.unwrap();
        assert_eq!(got, vec![e]);
    }

    #[tokio::test]
    async fn rows_with_mismatched_source_are_skipped() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        // Caller passes an edge with source wiki b under
        // a replace_source for wiki a. The mismatched edge
        // must be dropped, not silently inserted, so writers can't
        // accidentally pollute another owner's slice.
        store
            .replace_source(&w("a"), vec![edge(&w("b"), &f("x.rs"), "wiki_file_ref")])
            .await
            .unwrap();
        assert!(store.list_outbound(&w("b"), None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn target_index_used_for_backlinks_query() {
        // Functional check: thousands of unrelated edges, one
        // matching target, lookup by target_uri is
        // small. (We can't verify the index is HIT without
        // EXPLAIN QUERY PLAN, but a correctness test is fine.)
        let store = SqlitePageRefStore::new(Database::in_memory());
        let mut bulk = Vec::new();
        for i in 0..200 {
            bulk.push(edge(
                &w("noise"),
                &f(&format!("noise/{i}.rs")),
                "wiki_file_ref",
            ));
        }
        bulk.push(edge(&w("noise"), &f("target.rs"), "wiki_file_ref"));
        store.replace_source(&w("noise"), bulk).await.unwrap();
        let hits = store.list_backlinks(&f("target.rs"), None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, f("target.rs"));
    }
}
