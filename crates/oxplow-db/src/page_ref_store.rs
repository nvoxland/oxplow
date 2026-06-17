//! SQLite-backed unified cross-page reference graph.
//!
//! Each row is one directed edge `(source) --ref_type--> (target)`.
//! Per-subsystem writers own their `source_kind` rows (the wiki sync
//! owns all `wiki` rows, the task save path owns all
//! `task` rows, …). The standard write pattern is
//! [`SqlitePageRefStore::replace_source`]: delete every row whose
//! source matches, then re-insert the new edge set in one
//! transaction. The reader joins on `(target_kind, target_id)` for
//! backlinks or `(source_kind, source_id)` for outbound — both
//! covered by indexes.
//!
//! `kind` is denormalised next to `id` so kind-filtered lookups
//! ("all backlinks where target is a file") don't need a LIKE on a
//! synthetic combined column. Canonical ids match the frontend's
//! `TabRef.id` shape (e.g. `"wiki:architecture"`, `"wi-42"`,
//! `"file:src/app.rs"`, `"git-commit:abc123"`).

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::DomainError;

use crate::database::Database;

/// One directed edge in the page graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PageRefEdge {
    pub source_kind: String,
    pub source_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub ref_type: String,
    /// Optional JSON blob with edge-specific extras (line anchor,
    /// version pin, link sub-type, …). Opaque to the store.
    pub source_extra: Option<String>,
    /// Version data for edges pointing at a file-like target. Set
    /// by the writer via [`PageRefEdge::with_version`]; the store
    /// persists it verbatim. See V20 for column semantics.
    pub local_snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
}

impl PageRefEdge {
    pub fn new(
        source_kind: impl Into<String>,
        source_id: impl Into<String>,
        target_kind: impl Into<String>,
        target_id: impl Into<String>,
        ref_type: impl Into<String>,
    ) -> Self {
        Self {
            source_kind: source_kind.into(),
            source_id: source_id.into(),
            target_kind: target_kind.into(),
            target_id: target_id.into(),
            ref_type: ref_type.into(),
            source_extra: None,
            local_snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
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
        closest_git_version: Option<String>,
        git_version_exact: bool,
    ) -> Self {
        self.local_snapshot_id = Some(local_snapshot_id);
        self.closest_git_version = closest_git_version;
        self.git_version_exact = git_version_exact;
        self
    }
}

#[derive(Clone)]
pub struct SqlitePageRefStore {
    db: Database,
}

impl SqlitePageRefStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace every edge owned by `(source_kind, source_id)` with
    /// `edges`. Atomic. Idempotent: passing the same `edges` twice
    /// leaves the table unchanged. Edges in `edges` whose source
    /// doesn't match `(source_kind, source_id)` are silently skipped
    /// — callers shouldn't construct mixed batches but the store
    /// stays defensive.
    pub async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(crate::database::map_sql_err)?;
                tx.execute(
                    "DELETE FROM page_ref WHERE source_kind = ?1 AND source_id = ?2",
                    params![source_kind, source_id],
                )
                .map_err(crate::database::map_sql_err)?;
                for edge in edges {
                    if edge.source_kind != source_kind || edge.source_id != source_id {
                        continue;
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type,
                        source_extra, local_snapshot_id, closest_git_version,
                        git_version_exact)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            edge.source_kind,
                            edge.source_id,
                            edge.target_kind,
                            edge.target_id,
                            edge.ref_type,
                            edge.source_extra,
                            edge.local_snapshot_id,
                            edge.closest_git_version,
                            if edge.git_version_exact { 1 } else { 0 },
                        ],
                    )
                    .map_err(crate::database::map_sql_err)?;
                }
                tx.commit().map_err(crate::database::map_sql_err)
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
    ///   `local_snapshot_id` / `closest_git_version` /
    ///   `git_version_exact`. `source_extra` is updated to the new
    ///   value (line anchors, label overrides may legitimately
    ///   change without re-verifying the target).
    /// - Edge in `new \ existing` → INSERT with the new edge's
    ///   version data (caller stamps it via
    ///   `page_ref_projections::stamp_file_versions`).
    /// - Edge in `existing \ new` → DELETE.
    pub async fn merge_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(crate::database::map_sql_err)?;
                // Read existing PKs + version data for this source. The
                // unique key for an edge within a source is
                // (target_kind, target_id, ref_type).
                type Key = (String, String, String);
                type Existing = std::collections::HashMap<Key, (Option<i64>, Option<String>, bool)>;
                let existing: Existing = {
                    let mut stmt = tx
                        .prepare(
                            "SELECT target_kind, target_id, ref_type,
                                local_snapshot_id, closest_git_version,
                                git_version_exact
                         FROM page_ref
                         WHERE source_kind = ?1 AND source_id = ?2",
                        )
                        .map_err(crate::database::map_sql_err)?;
                    let rows = stmt
                        .query_map(params![&source_kind, &source_id], |r| {
                            let kind: String = r.get(0)?;
                            let id: String = r.get(1)?;
                            let rt: String = r.get(2)?;
                            let local: Option<i64> = r.get(3)?;
                            let git: Option<String> = r.get(4)?;
                            let exact: i64 = r.get(5)?;
                            Ok(((kind, id, rt), (local, git, exact != 0)))
                        })
                        .map_err(crate::database::map_sql_err)?;
                    let mut map = std::collections::HashMap::new();
                    for row in rows {
                        let (k, v) = row.map_err(crate::database::map_sql_err)?;
                        map.insert(k, v);
                    }
                    map
                };
                // Build the new key set for the post-merge delete step.
                let mut new_keys: std::collections::HashSet<Key> = std::collections::HashSet::new();
                for edge in &edges {
                    if edge.source_kind != source_kind || edge.source_id != source_id {
                        continue;
                    }
                    new_keys.insert((
                        edge.target_kind.clone(),
                        edge.target_id.clone(),
                        edge.ref_type.clone(),
                    ));
                }
                // Upsert each new edge. Match on PK; if existing,
                // preserve version data; otherwise stamp with the
                // caller-supplied version.
                for edge in edges {
                    if edge.source_kind != source_kind || edge.source_id != source_id {
                        continue;
                    }
                    let key = (
                        edge.target_kind.clone(),
                        edge.target_id.clone(),
                        edge.ref_type.clone(),
                    );
                    let (local, git, exact) = match existing.get(&key) {
                        Some(prev) => prev.clone(),
                        None => (
                            edge.local_snapshot_id,
                            edge.closest_git_version.clone(),
                            edge.git_version_exact,
                        ),
                    };
                    tx.execute(
                        "INSERT OR REPLACE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type,
                        source_extra, local_snapshot_id, closest_git_version,
                        git_version_exact)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            edge.source_kind,
                            edge.source_id,
                            edge.target_kind,
                            edge.target_id,
                            edge.ref_type,
                            edge.source_extra,
                            local,
                            git,
                            if exact { 1 } else { 0 },
                        ],
                    )
                    .map_err(crate::database::map_sql_err)?;
                }
                // Delete edges that existed but aren't in the new set.
                for key in existing.keys() {
                    if new_keys.contains(key) {
                        continue;
                    }
                    tx.execute(
                        "DELETE FROM page_ref
                     WHERE source_kind = ?1 AND source_id = ?2
                       AND target_kind = ?3 AND target_id = ?4
                       AND ref_type = ?5",
                        params![&source_kind, &source_id, &key.0, &key.1, &key.2],
                    )
                    .map_err(crate::database::map_sql_err)?;
                }
                tx.commit().map_err(crate::database::map_sql_err)
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
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type,
                        source_extra, local_snapshot_id, closest_git_version,
                        git_version_exact)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        edge.source_kind,
                        edge.source_id,
                        edge.target_kind,
                        edge.target_id,
                        edge.ref_type,
                        edge.source_extra,
                        edge.local_snapshot_id,
                        edge.closest_git_version,
                        if edge.git_version_exact { 1 } else { 0 },
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Per-target freshness rows for the wiki page identified by
    /// `slug`. Each row carries the captured snapshot pin from
    /// `page_ref` joined to the latest `snapshot.id` whose
    /// `file_snapshot.path` matches the target. Drives the wiki
    /// Freshness view.
    pub async fn list_wiki_file_freshness(
        &self,
        slug: &str,
    ) -> Result<Vec<(String, Option<i64>, Option<String>, bool, Option<i64>)>, DomainError> {
        let slug = slug.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT pr.target_id,
                            pr.local_snapshot_id,
                            pr.closest_git_version,
                            pr.git_version_exact,
                            (SELECT MAX(s.id)
                               FROM file_snapshot fs
                               JOIN snapshot s ON s.id = fs.snapshot_id
                              WHERE fs.path = pr.target_id) AS latest_snapshot_id
                       FROM page_ref pr
                      WHERE pr.source_kind = 'wiki'
                        AND pr.source_id = ?1
                        AND pr.target_kind = 'file'
                      ORDER BY pr.target_id ASC",
                )?;
                let rows = stmt.query_map(params![slug], |r| {
                    let path: String = r.get(0)?;
                    let local: Option<i64> = r.get(1)?;
                    let git: Option<String> = r.get(2)?;
                    let exact: i64 = r.get(3)?;
                    let latest: Option<i64> = r.get(4)?;
                    Ok((path, local, git, exact != 0, latest))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Every wiki page that has at least one **stale** file ref, with
    /// the stale paths. A file ref is stale when a `file_snapshot` for
    /// its path exists at a snapshot newer than the ref's captured pin
    /// (`local_snapshot_id`), or when the ref has no pin yet but the
    /// file has been snapshotted at all. This is the same rule the UI
    /// `list_wiki_freshness` reader applies per-ref (`latest > local`,
    /// or `latest present && local NULL`), expressed once here so an
    /// agent can find drifted pages without reading any body.
    ///
    /// Returns `(slug, path)` pairs ordered by slug then path; the
    /// caller groups them per page.
    pub async fn list_stale_wiki_pages(&self) -> Result<Vec<(String, String)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT pr.source_id, pr.target_id
                       FROM page_ref pr
                      WHERE pr.source_kind = 'wiki'
                        AND pr.target_kind = 'file'
                        AND EXISTS (
                            SELECT 1
                              FROM file_snapshot fs
                              JOIN snapshot s ON s.id = fs.snapshot_id
                             WHERE fs.path = pr.target_id
                               AND (pr.local_snapshot_id IS NULL
                                    OR s.id > pr.local_snapshot_id)
                        )
                      ORDER BY pr.source_id ASC, pr.target_id ASC",
                )?;
                let rows = stmt.query_map([], |r| {
                    let slug: String = r.get(0)?;
                    let path: String = r.get(1)?;
                    Ok((slug, path))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Re-stamp the version data on a single edge to the supplied
    /// snapshot pin. Used by the "Mark verified" UI affordances and
    /// by the wiki-update MCP call's `verified_refs` reconciliation.
    /// No-op if the matching row doesn't exist.
    // Each argument is doing distinct semantic work — the PK is 5
    // strings and the version pin is 3 fields. Bundling them into
    // a struct would just push the destructuring to every caller
    // without buying clarity.
    #[allow(clippy::too_many_arguments)]
    pub async fn restamp_edge_version(
        &self,
        source_kind: &str,
        source_id: &str,
        target_kind: &str,
        target_id: &str,
        ref_type: &str,
        local_snapshot_id: i64,
        closest_git_version: Option<String>,
        git_version_exact: bool,
    ) -> Result<(), DomainError> {
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        let target_kind = target_kind.to_string();
        let target_id = target_id.to_string();
        let ref_type = ref_type.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE page_ref
                        SET local_snapshot_id = ?6,
                            closest_git_version = ?7,
                            git_version_exact = ?8
                      WHERE source_kind = ?1 AND source_id = ?2
                        AND target_kind = ?3 AND target_id = ?4
                        AND ref_type = ?5",
                    params![
                        source_kind,
                        source_id,
                        target_kind,
                        target_id,
                        ref_type,
                        local_snapshot_id,
                        closest_git_version,
                        if git_version_exact { 1 } else { 0 },
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Like [`replace_source`] but restricted to a named slice
    /// identified by `ref_types`: only rows whose `ref_type` matches
    /// one of the supplied values are deleted, then `edges` is
    /// inserted. Used when a single source has multiple writers
    /// contributing different `ref_type`s — e.g. `task:wi-1`
    /// gets body-mention edges from the task upsert, link
    /// edges from the link store, and touched-file edges from the
    /// effort store. Each writer passes only its own ref_types so
    /// the others' rows survive.
    pub async fn replace_source_for_ref_types(
        &self,
        source_kind: &str,
        source_id: &str,
        ref_types: Vec<String>,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        if ref_types.is_empty() {
            return Ok(());
        }
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(crate::database::map_sql_err)?;
                let placeholders: Vec<String> =
                    (3..3 + ref_types.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "DELETE FROM page_ref
                 WHERE source_kind = ?1 AND source_id = ?2
                   AND ref_type IN ({})",
                    placeholders.join(",")
                );
                let mut params_vec: Vec<&dyn rusqlite::ToSql> =
                    Vec::with_capacity(2 + ref_types.len());
                params_vec.push(&source_kind);
                params_vec.push(&source_id);
                for rt in &ref_types {
                    params_vec.push(rt);
                }
                tx.execute(&sql, &params_vec[..])
                    .map_err(crate::database::map_sql_err)?;
                for edge in edges {
                    if edge.source_kind != source_kind || edge.source_id != source_id {
                        continue;
                    }
                    if !ref_types.contains(&edge.ref_type) {
                        continue;
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type,
                        source_extra, local_snapshot_id, closest_git_version,
                        git_version_exact)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            edge.source_kind,
                            edge.source_id,
                            edge.target_kind,
                            edge.target_id,
                            edge.ref_type,
                            edge.source_extra,
                            edge.local_snapshot_id,
                            edge.closest_git_version,
                            if edge.git_version_exact { 1 } else { 0 },
                        ],
                    )
                    .map_err(crate::database::map_sql_err)?;
                }
                tx.commit().map_err(crate::database::map_sql_err)
            })
            .await
    }

    /// Edges that point AT `(target_kind, target_id)` — i.e. the
    /// classic backlinks list. Order is by source-kind then
    /// source-id for stable rendering; callers can re-sort.
    pub async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let target_kind = target_kind.to_string();
        let target_id = target_id.to_string();
        let limit = limit.unwrap_or(500);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT source_kind, source_id, target_kind, target_id, ref_type,
                            source_extra, local_snapshot_id, closest_git_version,
                            git_version_exact
                     FROM page_ref
                     WHERE target_kind = ?1 AND target_id = ?2
                     ORDER BY source_kind, source_id, ref_type
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![target_kind, target_id, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Edges emitted FROM `(source_kind, source_id)` — the page's
    /// own outbound list.
    pub async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        let limit = limit.unwrap_or(500);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT source_kind, source_id, target_kind, target_id, ref_type,
                            source_extra, local_snapshot_id, closest_git_version,
                            git_version_exact
                     FROM page_ref
                     WHERE source_kind = ?1 AND source_id = ?2
                     ORDER BY target_kind, target_id, ref_type
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![source_kind, source_id, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageRefEdge> {
    let git_version_exact: i64 = row.get(8)?;
    Ok(PageRefEdge {
        source_kind: row.get(0)?,
        source_id: row.get(1)?,
        target_kind: row.get(2)?,
        target_id: row.get(3)?,
        ref_type: row.get(4)?,
        source_extra: row.get(5)?,
        local_snapshot_id: row.get(6)?,
        closest_git_version: row.get(7)?,
        git_version_exact: git_version_exact != 0,
    })
}

#[async_trait]
pub trait PageRefStore: Send + Sync {
    async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError>;
    async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
    async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
}

#[async_trait]
impl PageRefStore for SqlitePageRefStore {
    async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        SqlitePageRefStore::replace_source(self, source_kind, source_id, edges).await
    }
    async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_backlinks(self, target_kind, target_id, limit).await
    }
    async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_outbound(self, source_kind, source_id, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(s_kind: &str, s_id: &str, t_kind: &str, t_id: &str, ref_type: &str) -> PageRefEdge {
        PageRefEdge::new(s_kind, s_id, t_kind, t_id, ref_type)
    }

    #[tokio::test]
    async fn round_trip_single_edge() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(
            "wiki",
            "architecture",
            "file",
            "src/app.rs",
            "wiki_file_ref",
        );
        store
            .replace_source("wiki", "architecture", vec![e.clone()])
            .await
            .unwrap();

        let inbound = store
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert_eq!(inbound, vec![e.clone()]);

        let outbound = store
            .list_outbound("wiki", "architecture", None)
            .await
            .unwrap();
        assert_eq!(outbound, vec![e]);
    }

    #[tokio::test]
    async fn list_stale_wiki_pages_flags_drifted_and_unpinned_refs() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        // Seed a stream, two snapshots (id 100 < 200), and file
        // snapshots: stale.rs captured at the newer snapshot 200,
        // fresh.rs at 100.
        store
            .db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO streams
                       (id, kind, title, branch, branch_ref, branch_source,
                        worktree_path, created_at, updated_at)
                     VALUES (1,'primary','S','main','refs/heads/main','local',
                             '/tmp/s1','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                    [],
                )?;
                for id in [100i64, 200] {
                    conn.execute(
                        "INSERT INTO snapshot (id, stream_id, created_at)
                         VALUES (?1,1,'2026-01-01T00:00:00Z')",
                        params![id],
                    )?;
                }
                conn.execute(
                    "INSERT INTO file_snapshot
                       (stream_id, path, size_bytes, captured_at, storage, snapshot_id)
                     VALUES (1,'stale.rs',0,'2026-01-01T00:00:00Z','oxplow',200),
                            (1,'fresh.rs',0,'2026-01-01T00:00:00Z','oxplow',100)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        // p-stale: pinned at 100, file last snapshotted at 200 → stale.
        store
            .replace_source(
                "wiki",
                "p-stale",
                vec![edge("wiki", "p-stale", "file", "stale.rs", "wiki_file_ref")
                    .with_version(100, None, false)],
            )
            .await
            .unwrap();
        // p-fresh: pinned at 100, file last snapshotted at 100 → fresh.
        store
            .replace_source(
                "wiki",
                "p-fresh",
                vec![edge("wiki", "p-fresh", "file", "fresh.rs", "wiki_file_ref")
                    .with_version(100, None, false)],
            )
            .await
            .unwrap();
        // p-nopin: no pin at all, file has a snapshot → stale.
        store
            .replace_source(
                "wiki",
                "p-nopin",
                vec![edge("wiki", "p-nopin", "file", "stale.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();

        let stale = store.list_stale_wiki_pages().await.unwrap();
        assert_eq!(
            stale,
            vec![
                ("p-nopin".to_string(), "stale.rs".to_string()),
                ("p-stale".to_string(), "stale.rs".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn upsert_edge_inserts_then_overwrites_version() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(
            "wiki",
            "intro",
            "file",
            "crates/x/src/lib.rs",
            "wiki_file_ref",
        )
        .with_version(100, Some("aaaa".into()), true);
        store.upsert_edge(e).await.unwrap();
        let out = store.list_outbound("wiki", "intro", None).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].local_snapshot_id, Some(100));

        // Re-verifying overwrites the pin (unlike merge_source, which
        // preserves) — an explicit verification advances the pin.
        let e2 = edge(
            "wiki",
            "intro",
            "file",
            "crates/x/src/lib.rs",
            "wiki_file_ref",
        )
        .with_version(200, Some("bbbb".into()), false);
        store.upsert_edge(e2).await.unwrap();
        let out2 = store.list_outbound("wiki", "intro", None).await.unwrap();
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].local_snapshot_id, Some(200));
        assert_eq!(out2[0].closest_git_version.as_deref(), Some("bbbb"));
        assert!(!out2[0].git_version_exact);
    }

    #[tokio::test]
    async fn merge_source_preserves_existing_version() {
        // The first write stamps an edge at snapshot 100. A second
        // merge with the same edge (but a different version stamp)
        // must keep the original snapshot id and exact flag.
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e1 = edge("wiki", "intro", "file", "a.rs", "wiki_file_ref").with_version(
            100,
            Some("aaaa".into()),
            true,
        );
        store.merge_source("wiki", "intro", vec![e1]).await.unwrap();
        let e2 = edge("wiki", "intro", "file", "a.rs", "wiki_file_ref").with_version(
            200,
            Some("bbbb".into()),
            false,
        );
        store.merge_source("wiki", "intro", vec![e2]).await.unwrap();
        let out = store.list_outbound("wiki", "intro", None).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].local_snapshot_id, Some(100));
        assert_eq!(out[0].closest_git_version.as_deref(), Some("aaaa"));
        assert!(out[0].git_version_exact);
    }

    #[tokio::test]
    async fn merge_source_stamps_new_edges_and_deletes_missing() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let a =
            edge("wiki", "intro", "file", "a.rs", "wiki_file_ref").with_version(100, None, false);
        let b =
            edge("wiki", "intro", "file", "b.rs", "wiki_file_ref").with_version(100, None, false);
        store
            .merge_source("wiki", "intro", vec![a, b])
            .await
            .unwrap();
        // Second merge: drop b.rs, add c.rs. a.rs preserved; b
        // deleted; c.rs newly stamped at snapshot 200.
        let a2 =
            edge("wiki", "intro", "file", "a.rs", "wiki_file_ref").with_version(200, None, false);
        let c =
            edge("wiki", "intro", "file", "c.rs", "wiki_file_ref").with_version(200, None, false);
        store
            .merge_source("wiki", "intro", vec![a2, c])
            .await
            .unwrap();
        let out = store.list_outbound("wiki", "intro", None).await.unwrap();
        assert_eq!(out.len(), 2);
        let by_target: std::collections::HashMap<_, _> = out
            .iter()
            .map(|e| (e.target_id.clone(), e.local_snapshot_id))
            .collect();
        assert_eq!(by_target.get("a.rs"), Some(&Some(100)));
        assert_eq!(by_target.get("c.rs"), Some(&Some(200)));
        assert!(!by_target.contains_key("b.rs"));
    }

    #[tokio::test]
    async fn restamp_edge_version_updates_one_row() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let a =
            edge("wiki", "intro", "file", "a.rs", "wiki_file_ref").with_version(100, None, false);
        let b =
            edge("wiki", "intro", "file", "b.rs", "wiki_file_ref").with_version(100, None, false);
        store
            .merge_source("wiki", "intro", vec![a, b])
            .await
            .unwrap();
        store
            .restamp_edge_version(
                "wiki",
                "intro",
                "file",
                "a.rs",
                "wiki_file_ref",
                500,
                Some("eeee".into()),
                true,
            )
            .await
            .unwrap();
        let out = store.list_outbound("wiki", "intro", None).await.unwrap();
        let by_target: std::collections::HashMap<_, _> = out
            .iter()
            .map(|e| {
                (
                    e.target_id.clone(),
                    (
                        e.local_snapshot_id,
                        e.closest_git_version.clone(),
                        e.git_version_exact,
                    ),
                )
            })
            .collect();
        assert_eq!(
            by_target.get("a.rs"),
            Some(&(Some(500), Some("eeee".into()), true))
        );
        assert_eq!(by_target.get("b.rs"), Some(&(Some(100), None, false)));
    }

    #[tokio::test]
    async fn replace_source_is_idempotent() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let edges = vec![
            edge("wiki", "a", "file", "x.rs", "wiki_file_ref"),
            edge("wiki", "a", "task", "wi-1", "task_body_mention"),
        ];
        store
            .replace_source("wiki", "a", edges.clone())
            .await
            .unwrap();
        store
            .replace_source("wiki", "a", edges.clone())
            .await
            .unwrap();
        let out = store.list_outbound("wiki", "a", None).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn replace_source_drops_removed_edges() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(
                "wiki",
                "a",
                vec![
                    edge("wiki", "a", "file", "x.rs", "wiki_file_ref"),
                    edge("wiki", "a", "file", "y.rs", "wiki_file_ref"),
                ],
            )
            .await
            .unwrap();
        // Re-save with only y.rs — x.rs must vanish.
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "a", "file", "y.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        let inbound_x = store.list_backlinks("file", "x.rs", None).await.unwrap();
        let inbound_y = store.list_backlinks("file", "y.rs", None).await.unwrap();
        assert!(inbound_x.is_empty());
        assert_eq!(inbound_y.len(), 1);
    }

    #[tokio::test]
    async fn replace_source_doesnt_touch_other_sources() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "a", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        store
            .replace_source(
                "wiki",
                "b",
                vec![edge("wiki", "b", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        // Now replace `a` with empty — only `a`'s edges go.
        store.replace_source("wiki", "a", vec![]).await.unwrap();
        let inbound = store.list_backlinks("file", "x.rs", None).await.unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].source_id, "b");
    }

    #[tokio::test]
    async fn extra_payload_round_trips() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge("wiki", "a", "file", "x.rs", "wiki_file_ref")
            .with_extra(r#"{"line":42,"version":"@HEAD"}"#);
        store
            .replace_source("wiki", "a", vec![e.clone()])
            .await
            .unwrap();
        let got = store.list_outbound("wiki", "a", None).await.unwrap();
        assert_eq!(got, vec![e]);
    }

    #[tokio::test]
    async fn rows_with_mismatched_source_are_skipped() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        // Caller passes an edge with source ("wiki","b") under
        // a replace_source for ("wiki","a"). The mismatched edge
        // must be dropped, not silently inserted, so writers can't
        // accidentally pollute another owner's slice.
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "b", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        assert!(store
            .list_outbound("wiki", "b", None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn target_index_used_for_backlinks_query() {
        // Functional check: thousands of unrelated edges, one
        // matching target, lookup by (target_kind, target_id) is
        // small. (We can't verify the index is HIT without
        // EXPLAIN QUERY PLAN, but a correctness test is fine.)
        let store = SqlitePageRefStore::new(Database::in_memory());
        let mut bulk = Vec::new();
        for i in 0..200 {
            bulk.push(edge(
                "wiki",
                "noise",
                "file",
                &format!("noise/{i}.rs"),
                "wiki_file_ref",
            ));
        }
        bulk.push(edge("wiki", "noise", "file", "target.rs", "wiki_file_ref"));
        store.replace_source("wiki", "noise", bulk).await.unwrap();
        let hits = store
            .list_backlinks("file", "target.rs", None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target_id, "target.rs");
    }
}
