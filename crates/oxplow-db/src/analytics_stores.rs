//! Analytics-flavored stores: page_visit, usage_event, code_quality_*,
//! file_snapshot. They all share the same shape — append-mostly, with
//! recent-window queries — so they live together rather than each
//! getting its own file.

use async_trait::async_trait;
use oxplow_domain::vcs::Revision;
use oxplow_domain::vocabulary::{Vocabulary, VocabularyHandle};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{
    diff_trees, DomainError, FileChange, PageVisitId, StreamId, ThreadId, Timestamp, UsageEventId,
};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};
use crate::event_log_store::append_tx;
use crate::page_ref_projections::finding_edges;
use crate::snapshot_tree::{identities, manifest_hash, ContentHasher, SnapshotTree, TreeEntry};
use oxplow_domain::events::schema::{SnapshotTaken, SnapshotTakenV2, VcsHeadMoved, VcsHeadMovedV1};
use oxplow_domain::events::{Anchors, Envelope};
use oxplow_domain::refs::build::{commit_ref, snapshot_ref, stream_ref};
use oxplow_domain::snapshot::SnapshotTrigger;
use oxplow_domain::EffortId;

// ---------------- Page visits ----------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PageVisit {
    pub id: String,
    pub page_kind: String,
    pub page_id: String,
    /// Human-readable label captured at activation time — the same
    /// string the tab strip displays. NULL when none was captured (the
    /// renderer falls back to page_id).
    pub label: Option<String>,
    pub visited_at: Timestamp,
    pub duration_ms: Option<i64>,
    pub thread_id: Option<String>,
}

#[derive(Clone)]
pub struct SqlitePageVisitStore {
    db: Database,
}

impl SqlitePageVisitStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
pub trait PageVisitStore: Send + Sync {
    async fn record(
        &self,
        page_kind: &str,
        page_id: &str,
        label: Option<&str>,
        duration_ms: Option<i64>,
        thread_id: Option<&str>,
    ) -> Result<PageVisit, DomainError>;
    /// Recent visits, optionally scoped to one thread. `None` returns
    /// every visit across threads (the global view).
    async fn list_recent(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<PageVisit>, DomainError>;
    /// Top visited (kind, id) tuples by visit count, optionally scoped to
    /// one thread.
    async fn list_top(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<(String, String, i64)>, DomainError>;
    async fn forget_page(&self, page_kind: &str, page_id: &str) -> Result<(), DomainError>;
    /// Distinct (page_kind, page_id) tuples ordered by most recent visit
    /// — drives the "frequent" rail.
    async fn list_frequent(&self, limit: usize) -> Result<Vec<PageVisit>, DomainError>;
}

#[async_trait]
impl PageVisitStore for SqlitePageVisitStore {
    async fn record(
        &self,
        page_kind: &str,
        page_id: &str,
        label: Option<&str>,
        duration_ms: Option<i64>,
        thread_id: Option<&str>,
    ) -> Result<PageVisit, DomainError> {
        let page_kind = page_kind.to_string();
        let page_id = page_id.to_string();
        let label = label.map(|s| s.to_string());
        let thread_id = thread_id.map(|s| s.to_string());
        self.db
            .call(move |conn| {
                let now = Timestamp::now();
                // thread_id arrives as a prefixed string ("thr3"); the
                // column is INTEGER, so store the raw rowid.
                let thread_id_val: Option<i64> = thread_id
                    .as_deref()
                    .and_then(ThreadId::try_from_str)
                    .map(|t| t.value());
                conn.execute(
                    "INSERT INTO page_visit (page_kind, page_id, label, visited_at, duration_ms, thread_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![page_kind, page_id, label, ts_to_string(now), duration_ms, thread_id_val],
                )?;
                let id = PageVisitId::new(conn.last_insert_rowid()).to_string();
                Ok(PageVisit {
                    id,
                    page_kind,
                    page_id,
                    label,
                    visited_at: now,
                    duration_ms,
                    thread_id,
                })
            })
            .await
    }

    async fn list_recent(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<PageVisit>, DomainError> {
        let thread_id = thread_id.map(|s| s.to_string());
        self.db
            .call(move |conn| {
                let mut stmt = if thread_id.is_some() {
                    conn.prepare(
                        "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                         FROM page_visit
                         WHERE thread_id = ?2
                         ORDER BY visited_at DESC LIMIT ?1",
                    )?
                } else {
                    conn.prepare(
                        "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                         FROM page_visit
                         ORDER BY visited_at DESC LIMIT ?1",
                    )?
                };
                let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<PageVisit> {
                    let id = PageVisitId::new(row.get::<_, i64>(0)?).to_string();
                    let page_kind: String = row.get(1)?;
                    let page_id: String = row.get(2)?;
                    let label: Option<String> = row.get(3)?;
                    let visited_at: String = row.get(4)?;
                    let duration_ms: Option<i64> = row.get(5)?;
                    let thread_id = row
                        .get::<_, Option<i64>>(6)?
                        .map(|t| ThreadId::new(t).to_string());
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(PageVisit {
                        id,
                        page_kind,
                        page_id,
                        label,
                        visited_at: string_to_ts(&visited_at).map_err(map_err)?,
                        duration_ms,
                        thread_id,
                    })
                };
                if let Some(tid) = thread_id
                    .as_deref()
                    .and_then(ThreadId::try_from_str)
                    .map(|t| t.value())
                {
                    let rows: rusqlite::Result<Vec<_>> = stmt
                        .query_map(params![limit as i64, tid], map_row)?
                        .collect();
                    rows
                } else {
                    let rows: rusqlite::Result<Vec<_>> =
                        stmt.query_map(params![limit as i64], map_row)?.collect();
                    rows
                }
            })
            .await
    }

    async fn list_top(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<(String, String, i64)>, DomainError> {
        let thread_id = thread_id.map(|s| s.to_string());
        self.db
            .call(move |conn| {
                let map_row = |row: &rusqlite::Row<'_>| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                };
                if let Some(tid) = thread_id
                    .as_deref()
                    .and_then(ThreadId::try_from_str)
                    .map(|t| t.value())
                {
                    let mut stmt = conn.prepare(
                        "SELECT page_kind, page_id, COUNT(*) AS visits
                         FROM page_visit
                         WHERE thread_id = ?2
                         GROUP BY page_kind, page_id
                         ORDER BY visits DESC
                         LIMIT ?1",
                    )?;
                    let rows: rusqlite::Result<Vec<_>> = stmt
                        .query_map(params![limit as i64, tid], map_row)?
                        .collect();
                    rows
                } else {
                    let mut stmt = conn.prepare(
                        "SELECT page_kind, page_id, COUNT(*) AS visits
                         FROM page_visit
                         GROUP BY page_kind, page_id
                         ORDER BY visits DESC
                         LIMIT ?1",
                    )?;
                    let rows: rusqlite::Result<Vec<_>> =
                        stmt.query_map(params![limit as i64], map_row)?.collect();
                    rows
                }
            })
            .await
    }

    async fn forget_page(&self, page_kind: &str, page_id: &str) -> Result<(), DomainError> {
        let page_kind = page_kind.to_string();
        let page_id = page_id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM page_visit WHERE page_kind = ?1 AND page_id = ?2",
                    params![page_kind, page_id],
                )?;
                Ok(())
            })
            .await
    }

    async fn list_frequent(&self, limit: usize) -> Result<Vec<PageVisit>, DomainError> {
        self.db
            .call(move |conn| {
                // Most-recent visit per page, ordered by visit count desc.
                let mut stmt = conn.prepare(
                    "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                     FROM page_visit pv
                     WHERE id = (
                         SELECT id FROM page_visit pv2
                         WHERE pv2.page_kind = pv.page_kind AND pv2.page_id = pv.page_id
                         ORDER BY visited_at DESC LIMIT 1
                     )
                     ORDER BY (
                         SELECT COUNT(*) FROM page_visit pv3
                         WHERE pv3.page_kind = pv.page_kind AND pv3.page_id = pv.page_id
                     ) DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], |row| {
                    let id = PageVisitId::new(row.get::<_, i64>(0)?).to_string();
                    let page_kind: String = row.get(1)?;
                    let page_id: String = row.get(2)?;
                    let label: Option<String> = row.get(3)?;
                    let visited_at: String = row.get(4)?;
                    let duration_ms: Option<i64> = row.get(5)?;
                    let thread_id = row
                        .get::<_, Option<i64>>(6)?
                        .map(|t| ThreadId::new(t).to_string());
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(PageVisit {
                        id,
                        page_kind,
                        page_id,
                        label,
                        visited_at: string_to_ts(&visited_at).map_err(map_err)?,
                        duration_ms,
                        thread_id,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

// ---------------- Usage events ----------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsageEvent {
    pub id: String,
    pub kind: String,
    pub payload_json: String,
    pub occurred_at: Timestamp,
}

/// Per-key aggregation of usage events. Returned by
/// `SqliteUsageStore::list_recent_rollup` for callers that want
/// "most-recently-touched X" lists rather than the raw event log
/// (e.g. the WikiActivityBar's recent-files strip).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsageRollup {
    pub kind: String,
    pub key: String,
    pub last_at: Timestamp,
    pub count: u32,
}

#[derive(Clone)]
pub struct SqliteUsageStore {
    db: Database,
}

impl SqliteUsageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(
        &self,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<UsageEvent, DomainError> {
        let kind = kind.to_string();
        let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        self.db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "INSERT INTO usage_event (kind, payload_json, occurred_at)
                     VALUES (?1, ?2, ?3)",
                    params![kind, payload_json, ts_to_string(now)],
                )?;
                let id = UsageEventId::new(conn.last_insert_rowid()).to_string();
                Ok(UsageEvent {
                    id,
                    kind,
                    payload_json,
                    occurred_at: now,
                })
            })
            .await
    }

    /// Group recent events of a single `kind` by the per-row key
    /// extracted from `payload_json`. The extraction tries the same
    /// candidate fields that `commands::usage::extract_key` does
    /// (`key` → `slug` → `path` → `id` → `itemId` / `item_id` →
    /// `noteId` / `note_id`); rows whose payload yields no key are
    /// dropped. When `stream_id` is `Some`, only events whose payload
    /// includes that `streamId` (or `stream_id`) are counted.
    pub async fn list_recent_rollup(
        &self,
        kind: &str,
        stream_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<UsageRollup>, DomainError> {
        let kind = kind.to_string();
        let stream_id = stream_id.map(|s| s.to_string());
        self.db
            .call(move |conn| {
                // COALESCE over the canonical key fields a payload
                // names its thing by (`key`, `slug`, `path`, `id`, …).
                let stream_filter = if stream_id.is_some() {
                    "AND COALESCE(json_extract(payload_json, '$.streamId'), \
                                  json_extract(payload_json, '$.stream_id')) = ?3"
                } else {
                    ""
                };
                let sql = format!(
                    "SELECT \
                       COALESCE( \
                         json_extract(payload_json, '$.key'), \
                         json_extract(payload_json, '$.slug'), \
                         json_extract(payload_json, '$.path'), \
                         json_extract(payload_json, '$.id'), \
                         json_extract(payload_json, '$.itemId'), \
                         json_extract(payload_json, '$.item_id'), \
                         json_extract(payload_json, '$.noteId'), \
                         json_extract(payload_json, '$.note_id') \
                       ) AS key, \
                       MAX(occurred_at) AS last_at, \
                       COUNT(*) AS cnt \
                     FROM usage_event \
                     WHERE kind = ?1 {stream_filter} \
                     GROUP BY key \
                     HAVING key IS NOT NULL AND key != '' \
                     ORDER BY last_at DESC \
                     LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql)?;
                let map_err = |e: DomainError| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                };
                let collect = |row: &rusqlite::Row| -> rusqlite::Result<UsageRollup> {
                    let key: String = row.get(0)?;
                    let last_at: String = row.get(1)?;
                    let cnt: i64 = row.get(2)?;
                    Ok(UsageRollup {
                        kind: kind.clone(),
                        key,
                        last_at: string_to_ts(&last_at).map_err(map_err)?,
                        count: cnt.max(0) as u32,
                    })
                };
                let rows: Vec<UsageRollup> = if let Some(sid) = stream_id.as_deref() {
                    stmt.query_map(params![kind, limit as i64, sid], collect)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(params![kind, limit as i64], collect)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<UsageEvent>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, payload_json, occurred_at FROM usage_event
                     ORDER BY occurred_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], |row| {
                    let id = UsageEventId::new(row.get::<_, i64>(0)?).to_string();
                    let kind: String = row.get(1)?;
                    let payload_json: String = row.get(2)?;
                    let occurred_at: String = row.get(3)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(UsageEvent {
                        id,
                        kind,
                        payload_json,
                        occurred_at: string_to_ts(&occurred_at).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

// ---------------- Code quality ----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CodeQualityScanStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CodeQualityScan {
    pub id: i64,
    pub tool: String,
    pub scope: String,
    pub status: CodeQualityScanStatus,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub error: Option<String>,
    /// The revision the scan read (`working`, `snap:<id>`,
    /// `git:<rev>`; `oxplow_domain::vcs::Revision`).
    pub revision: String,
    /// File filter applied: `"all"` or `"explicit:<sha-of-paths>"`.
    /// Backfilled to `"all"` for pre-V9 rows.
    pub file_filter: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CodeQualityFinding {
    pub id: i64,
    pub scan_id: i64,
    pub path: String,
    pub start_line: i32,
    pub end_line: i32,
    pub kind: String,
    pub metric_value: f64,
    pub extra_json: Option<String>,
}

fn row_to_scan(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodeQualityScan> {
    let id: i64 = row.get(0)?;
    let tool: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let status: String = row.get(3)?;
    let started_at: String = row.get(4)?;
    let ended_at: Option<String> = row.get(5)?;
    let error: Option<String> = row.get(6)?;
    let revision: String = row.get(7)?;
    let file_filter: Option<String> = row.get(8)?;
    let status = match status.as_str() {
        "pending" => CodeQualityScanStatus::Pending,
        "running" => CodeQualityScanStatus::Running,
        "done" => CodeQualityScanStatus::Done,
        _ => CodeQualityScanStatus::Failed,
    };
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(CodeQualityScan {
        id,
        tool,
        scope,
        status,
        started_at: string_to_ts(&started_at).map_err(map_err)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        error,
        revision,
        file_filter: file_filter.unwrap_or_else(|| "all".into()),
    })
}

#[derive(Clone)]
pub struct SqliteCodeQualityStore {
    db: Database,
}

impl SqliteCodeQualityStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Start a scan of `revision` (`oxplow_domain::vcs::Revision`'s
    /// string) with `file_filter` (`all` / `explicit:<hash>`), so its
    /// results never pass for another version's.
    pub async fn create_scan(
        &self,
        tool: &str,
        scope: &str,
        revision: &str,
        file_filter: &str,
    ) -> Result<i64, DomainError> {
        let tool = tool.to_string();
        let scope = scope.to_string();
        let revision = revision.to_string();
        let filter = file_filter.to_string();
        self.db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "INSERT INTO code_quality_scan
                       (tool, scope, status, started_at, revision, file_filter)
                     VALUES (?1, ?2, 'pending', ?3, ?4, ?5)",
                    params![tool, scope, ts_to_string(now), revision, filter],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }

    pub async fn finish_scan(
        &self,
        id: i64,
        status: CodeQualityScanStatus,
        error: Option<String>,
    ) -> Result<(), DomainError> {
        let status_str = match status {
            CodeQualityScanStatus::Pending => "pending",
            CodeQualityScanStatus::Running => "running",
            CodeQualityScanStatus::Done => "done",
            CodeQualityScanStatus::Failed => "failed",
        };
        self.db
            .call(move |conn| {
                let now = Timestamp::now();
                conn.execute(
                    "UPDATE code_quality_scan SET status = ?2, ended_at = ?3, error = ?4 WHERE id = ?1",
                    params![id, status_str, ts_to_string(now), error],
                )?;
                Ok(())
            })
            .await
    }

    /// Finish scan `scan_id` as done with its `findings`, in one write
    /// transaction (so one commit, one `ModelsChanged`): each finding, its
    /// file edge in `page_ref`, the scan's status — and the older finished
    /// scans of the same tool and scope it replaces, with their findings
    /// and edges. Readers only ever want a scope's latest scan; a running
    /// one is left alone.
    pub async fn finish_scan_with_findings(
        &self,
        scan_id: i64,
        findings: Vec<CodeQualityFinding>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                let sql = crate::database::map_sql_err;
                for f in &findings {
                    tx.execute(
                        "INSERT INTO code_quality_finding
                           (scan_id, path, start_line, end_line, kind, metric_value, extra_json)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            scan_id,
                            f.path,
                            f.start_line,
                            f.end_line,
                            f.kind,
                            f.metric_value,
                            f.extra_json,
                        ],
                    )
                    .map_err(sql)?;
                    let id = tx.last_insert_rowid().to_string();
                    crate::page_ref_store::replace_source_tx(
                        tx,
                        "finding",
                        &id,
                        finding_edges(&id, &f.path),
                    )?;
                }
                tx.execute(
                    "UPDATE code_quality_scan SET status = 'done', ended_at = ?2, error = NULL
                     WHERE id = ?1",
                    params![scan_id, ts_to_string(Timestamp::now())],
                )
                .map_err(sql)?;
                // The scans this one replaces: same tool and scope, older,
                // finished. Their findings cascade; their edges don't.
                let superseded = "SELECT o.id FROM code_quality_scan o
                     JOIN code_quality_scan n ON n.id = ?1
                     WHERE o.tool = n.tool AND o.scope = n.scope AND o.id < n.id
                       AND o.status <> 'running'";
                tx.execute(
                    &format!(
                        "DELETE FROM page_ref WHERE source_kind = 'finding' AND source_id IN (
                           SELECT CAST(f.id AS TEXT) FROM code_quality_finding f
                           WHERE f.scan_id IN ({superseded}))"
                    ),
                    params![scan_id],
                )
                .map_err(sql)?;
                tx.execute(
                    &format!("DELETE FROM code_quality_scan WHERE id IN ({superseded})"),
                    params![scan_id],
                )
                .map_err(sql)?;
                Ok(())
            })
            .await
    }

    /// All findings across every scan as `(rowid, path)`. Used by
    /// the page-ref backfill — each finding row owns one
    /// `(finding:<rowid>) -> (file:<path>)` edge.
    pub async fn list_all_findings_for_backfill(&self) -> Result<Vec<(i64, String)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare("SELECT id, path FROM code_quality_finding")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn list_scans(&self, limit: usize) -> Result<Vec<CodeQualityScan>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, tool, scope, status, started_at, ended_at, error,
                            revision, file_filter
                     FROM code_quality_scan ORDER BY started_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], row_to_scan)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn list_findings(
        &self,
        scan_id: i64,
    ) -> Result<Vec<CodeQualityFinding>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, scan_id, path, start_line, end_line, kind, metric_value, extra_json
                     FROM code_quality_finding WHERE scan_id = ?1 ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![scan_id], |row| {
                    Ok(CodeQualityFinding {
                        id: row.get(0)?,
                        scan_id: row.get(1)?,
                        path: row.get(2)?,
                        start_line: row.get(3)?,
                        end_line: row.get(4)?,
                        kind: row.get(5)?,
                        metric_value: row.get(6)?,
                        extra_json: row.get(7)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Fetch a single finding by its integer id — the canonical
    /// `finding:<id>` ref. Used by the typed ref-resolver to hydrate a
    /// finding into a label (kind) + location for agent context.
    pub async fn get_finding(&self, id: i64) -> Result<Option<CodeQualityFinding>, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT id, scan_id, path, start_line, end_line, kind, metric_value, extra_json
                     FROM code_quality_finding WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(CodeQualityFinding {
                            id: row.get(0)?,
                            scan_id: row.get(1)?,
                            path: row.get(2)?,
                            start_line: row.get(3)?,
                            end_line: row.get(4)?,
                            kind: row.get(5)?,
                            metric_value: row.get(6)?,
                            extra_json: row.get(7)?,
                        })
                    },
                )
                .optional()
            })
            .await
    }
}

// ---------------- File snapshots ----------------

/// Where a captured file's bytes live — the explicit `file_snapshot.storage`
/// discriminator (V37). Replaces the old implicit `(blob_hash NULL?,
/// oversize?)` 2-bit encoding. The `blob_hash` column means different
/// things per variant:
/// - [`Oxplow`](SnapshotStorage::Oxplow): `blob_hash` is an xxh3-128, bytes
///   in `.oxplow/snapshots/objects/`.
/// - [`Git`](SnapshotStorage::Git): `blob_hash` is a **git blob OID**, bytes
///   recovered on demand from the git object db (`git cat-file` / libgit2
///   `find_blob`). The capture path never copied them — a clean checkout
///   reuses git's own storage.
/// - [`Oversize`](SnapshotStorage::Oversize): `blob_hash` is NULL; the file
///   was too big to hash, only `size_bytes` + `mtime_ms` are tracked.
/// - [`Deleted`](SnapshotStorage::Deleted): `blob_hash` is NULL; a tombstone
///   row marking the path gone as of this snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStorage {
    Oxplow,
    Git,
    Oversize,
    Deleted,
}

impl SnapshotStorage {
    /// The TEXT value persisted in `file_snapshot.storage`.
    pub fn as_db_str(self) -> &'static str {
        match self {
            SnapshotStorage::Oxplow => "oxplow",
            SnapshotStorage::Git => "git",
            SnapshotStorage::Oversize => "oversize",
            SnapshotStorage::Deleted => "deleted",
        }
    }

    /// Parse the persisted TEXT value. Unknown values fall back to
    /// `Oxplow` (the historical default) so a forward-incompatible row
    /// never panics a read.
    pub fn from_db_str(s: &str) -> SnapshotStorage {
        match s {
            "git" => SnapshotStorage::Git,
            "oversize" => SnapshotStorage::Oversize,
            "deleted" => SnapshotStorage::Deleted,
            _ => SnapshotStorage::Oxplow,
        }
    }

    /// True when `blob_hash` addresses real bytes (oxplow blob store or
    /// git odb) — i.e. the content can be read back.
    pub fn has_bytes(self) -> bool {
        matches!(self, SnapshotStorage::Oxplow | SnapshotStorage::Git)
    }

    /// True for the oversize metadata-only class.
    pub fn is_oversize(self) -> bool {
        matches!(self, SnapshotStorage::Oversize)
    }

    /// True for a deletion tombstone (path absent as of this snapshot).
    pub fn is_deleted(self) -> bool {
        matches!(self, SnapshotStorage::Deleted)
    }
}

/// A readable handle to a captured file's bytes: the storage class plus
/// the `blob_hash` that addresses the content under it. Only produced
/// for rows whose storage [`has_bytes`](SnapshotStorage::has_bytes)
/// (oxplow / git). The read seam (`oxplow_app`) switches on `storage`
/// to fetch from the blob store or the git odb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotContentRef {
    pub storage: SnapshotStorage,
    pub hash: String,
}

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileSnapshot> {
    let id: i64 = row.get(0)?;
    let stream_id: i64 = row.get(1)?;
    let path: String = row.get(2)?;
    let blob_hash: Option<String> = row.get(3)?;
    let size_bytes: i64 = row.get(4)?;
    let captured_at: String = row.get(5)?;
    let storage: String = row.get(6)?;
    // snapshot_id / mtime_ms only present when the SELECT asks for
    // them (V13 / V15+); older 7-column callers see them as missing
    // and we treat that as None.
    let snapshot_id: Option<i64> = row.get(7).ok().flatten();
    let mtime_ms: Option<i64> = row.get(8).ok().flatten();
    // Present only when the SELECT names it (V96+).
    let content_hash: Option<String> = row.get("content_hash").ok().flatten();
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(FileSnapshot {
        id,
        stream_id: StreamId::new(stream_id),
        path,
        blob_hash,
        size_bytes,
        captured_at: string_to_ts(&captured_at).map_err(map_err)?,
        storage: SnapshotStorage::from_db_str(&storage),
        snapshot_id,
        mtime_ms,
        content_hash,
    })
}

/// Aggregate created/modified/deleted counts for the file rows
/// captured under one snapshot. Derived by comparing each child
/// row's `blob_hash` to the most-recent prior row for the same
/// `(stream_id, path)`. Powers the Local History dashboard's
/// per-snapshot stats column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
pub struct SnapshotStats {
    pub created: i64,
    pub modified: i64,
    pub deleted: i64,
    pub total: i64,
}

/// One row per file captured under a snapshot, in the shape the
/// renderer's change-analysis pipeline expects. `status` mirrors
/// `BranchChangeEntry`'s set (`added`/`modified`/`deleted`) so the
/// shared SummaryCard / ChangeAnalysisPanel can render snapshot
/// changes alongside git ones. `current_file_id` is the row in
/// `file_snapshot` captured for this snapshot; `prior_file_id` is
/// the most recent prior capture of the same `(stream_id, path)`,
/// used to pull the "before" blob bytes for diff + function
/// analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SnapshotChangeEntry {
    pub path: String,
    pub status: String,
    pub current_file_id: i64,
    pub prior_file_id: Option<i64>,
    pub oversize: bool,
}

/// `snapshot` row — one per `request_snapshot()` call that had
/// dirty files. Groups the `file_snapshot` rows captured in that
/// batch. See [[crates/oxplow-db/migrations/V13__snapshot.sql]].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Snapshot {
    pub id: i64,
    pub stream_id: StreamId,
    pub created_at: Timestamp,
    pub file_count: i64,
    /// The VCS revision this snapshot's tree equals (`git:<sha>`).
    /// Populated only when the workspace was clean at the head at
    /// capture time; `None` when it was dirty or isn't under version
    /// control. Not unique — several snapshots can share one revision
    /// when local history captures files the VCS doesn't track.
    #[specta(type = Option<String>)]
    pub revision: Option<Revision>,
    /// The branch the workspace was on when this snapshot was captured
    /// (e.g. `main`). `None` for pre-V42 rows, a detached head, or a
    /// directory not under version control.
    pub branch: Option<String>,
    /// Whole-tree identity (V96): the xxh3-128 of the sorted manifest of
    /// the reconstructed tree ([`crate::snapshot_tree::manifest_hash`]).
    /// Two snapshots with equal `tree_hash` hold the same files. `None`
    /// on snapshots taken before V96.
    pub tree_hash: Option<String>,
    /// What the take that CREATED this snapshot recorded (its first
    /// `snapshot_op`, P2.11): the snapshot it grew from — the "previous"
    /// a single-snapshot diff starts at — why it was taken, and whether
    /// it ran over its budget. `None` / `false` for a snapshot with no op.
    pub parent_snapshot_id: Option<i64>,
    pub trigger: Option<SnapshotTrigger>,
    pub over_budget: bool,
}

/// Most-recent stat (hash + size + mtime) for a single path. The
/// startup sweep uses this to short-circuit the read+hash pass when
/// `(size_bytes, mtime_ms)` match the file on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestStat {
    pub blob_hash: Option<String>,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
    pub storage: SnapshotStorage,
    /// The row's content identity when known (V96): always for `oxplow`,
    /// lazily for `git`, never for `oversize`/`deleted`.
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FileSnapshot {
    pub id: i64,
    pub stream_id: StreamId,
    pub path: String,
    /// The storage ADDRESS: xxh3-128 (storage = oxplow) or git blob OID
    /// (storage = git); NULL for oversize / deleted rows. Compare
    /// [`Self::content_hash`], not this.
    pub blob_hash: Option<String>,
    pub size_bytes: i64,
    pub captured_at: Timestamp,
    /// Where the bytes live — see [`SnapshotStorage`]. Replaces the old
    /// `oversize: bool` field; deletion tombstones are
    /// `SnapshotStorage::Deleted`.
    pub storage: SnapshotStorage,
    /// `snapshot.id` this row was captured under, or `None` for
    /// pre-V13 rows that predate the snapshot grouping table.
    pub snapshot_id: Option<i64>,
    /// File mtime in unix milliseconds at capture time. NULL for
    /// rows written before V15 added the column. The startup sweep
    /// uses `(size_bytes, mtime_ms)` as a fast equality check: if
    /// both match the current stat, the file is presumed unchanged
    /// and the bytes aren't re-read or re-hashed.
    pub mtime_ms: Option<i64>,
    /// The content identity (V96): xxh3-128 of the bytes, whatever the
    /// storage class. `blob_hash` is only the address. [`capture_batch`]
    /// fills it from `blob_hash` for `oxplow` rows; `git` rows get it
    /// lazily ([`SqliteSnapshotStore::with_content_hasher`]).
    ///
    /// [`capture_batch`]: SqliteSnapshotStore::capture_batch
    pub content_hash: Option<String>,
}

/// One commit-stamped snapshot row — a point where a stream's worktree WAS
/// exactly a commit. The metric-ancestry resolver anchors dirty captures to
/// these (tsk102).
#[derive(Debug, Clone, PartialEq)]
pub struct StampedSnapshot {
    /// The snapshot row id — captures reference it (`capture.snapshot_id`),
    /// and a capture's OWN snapshot being stamped is the primary anchor (the
    /// re-stamp flow leaves `created_at` BEFORE the captures that ran on it).
    pub id: i64,
    pub stream_id: i64,
    pub branch: Option<String>,
    /// The revision's id in its VCS (a commit sha) — the value part of
    /// `snapshot.revision`.
    pub commit: String,
    pub created_at: Timestamp,
}

/// Every row captured under snapshot `?1`, classified against the most
/// recent prior row for the same `(stream_id, path)` by content identity:
/// `kind` is `added` / `modified` / `deleted`, or NULL when the row is not
/// a change (same bytes as before, or a tombstone for a path already
/// absent). Columns: `id, path, storage, prior_id, kind`.
const CLASSIFIED_CHANGES: &str = "
    SELECT c.id, c.path, c.storage, c.prior_id,
           CASE
             WHEN c.storage = 'deleted' THEN
               CASE WHEN p.storage IS NULL OR p.storage = 'deleted' THEN NULL ELSE 'deleted' END
             WHEN p.storage IS NULL OR p.storage = 'deleted' THEN 'added'
             WHEN c.ident = COALESCE(p.content_hash,
                            CASE p.storage
                              WHEN 'oxplow' THEN p.blob_hash
                              WHEN 'git' THEN 'git:' || p.blob_hash
                              WHEN 'oversize' THEN 'oversize:' || p.size_bytes || ':' || COALESCE(p.mtime_ms, 0)
                            END) THEN NULL
             ELSE 'modified'
           END AS kind
      FROM (
        SELECT f.id, f.path, f.storage, COALESCE(f.content_hash,
                            CASE f.storage
                              WHEN 'oxplow' THEN f.blob_hash
                              WHEN 'git' THEN 'git:' || f.blob_hash
                              WHEN 'oversize' THEN 'oversize:' || f.size_bytes || ':' || COALESCE(f.mtime_ms, 0)
                            END) AS ident,
               (SELECT q.id FROM file_snapshot q
                 WHERE q.stream_id = f.stream_id AND q.path = f.path AND q.id < f.id
                 ORDER BY q.id DESC LIMIT 1) AS prior_id
          FROM file_snapshot f
         WHERE f.snapshot_id = ?1
      ) c
      LEFT JOIN file_snapshot p ON p.id = c.prior_id";

/// [`SqliteSnapshotStore::tree_at`] on a connection (also used inside
/// transactions that need the tree).
pub(crate) fn tree_at_conn(
    conn: &rusqlite::Connection,
    snapshot_id: i64,
) -> rusqlite::Result<SnapshotTree> {
    let stream_id: Option<i64> = conn
        .query_row(
            "SELECT stream_id FROM snapshot WHERE id = ?1",
            params![snapshot_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(stream_id) = stream_id else {
        return Ok(SnapshotTree::new());
    };
    let mut stmt = conn.prepare(
        "SELECT path, blob_hash, storage, size_bytes, mtime_ms, content_hash FROM (
            SELECT path, blob_hash, storage, size_bytes, mtime_ms, content_hash,
                   ROW_NUMBER() OVER (
                     PARTITION BY path ORDER BY snapshot_id DESC, id DESC
                   ) AS rn
            FROM file_snapshot
            WHERE stream_id = ?1
              AND snapshot_id IS NOT NULL
              AND snapshot_id <= ?2
         ) WHERE rn = 1 AND storage <> 'deleted'",
    )?;
    let rows = stmt.query_map(params![stream_id, snapshot_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            TreeEntry {
                address: row.get(1)?,
                storage: SnapshotStorage::from_db_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                mtime_ms: row.get(4)?,
                content_hash: row.get(5)?,
            },
        ))
    })?;
    rows.collect()
}

/// Insert `file_snapshot` rows, returning their ids in order. An `oxplow`
/// row's address IS its content hash, so `content_hash` is filled from
/// `blob_hash` when the caller didn't set it.
fn insert_rows_tx(
    conn: &rusqlite::Connection,
    rows: &[FileSnapshot],
) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "INSERT INTO file_snapshot
           (stream_id, path, blob_hash, size_bytes, captured_at, storage,
            snapshot_id, mtime_ms, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    let mut ids = Vec::with_capacity(rows.len());
    for snap in rows {
        let content_hash = snap.content_hash.clone().or_else(|| {
            (snap.storage == SnapshotStorage::Oxplow)
                .then(|| snap.blob_hash.clone())
                .flatten()
        });
        stmt.execute(params![
            snap.stream_id.value(),
            snap.path,
            snap.blob_hash,
            snap.size_bytes,
            ts_to_string(snap.captured_at),
            snap.storage.as_db_str(),
            snap.snapshot_id,
            snap.mtime_ms,
            content_hash,
        ])?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

/// The snapshot the stream's worktree is at: the latest op's snapshot
/// (the op log is complete since V97's backfill), else the newest
/// snapshot row.
pub(crate) fn current_snapshot_tx(
    conn: &rusqlite::Connection,
    stream_id: StreamId,
) -> rusqlite::Result<Option<i64>> {
    let from_ops: Option<i64> = conn
        .query_row(
            "SELECT snapshot_id FROM snapshot_op WHERE stream_id = ?1 ORDER BY seq DESC LIMIT 1",
            params![stream_id.value()],
            |r| r.get(0),
        )
        .optional()?;
    if from_ops.is_some() {
        return Ok(from_ops);
    }
    conn.query_row(
        "SELECT id FROM snapshot WHERE stream_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
        params![stream_id.value()],
        |r| r.get(0),
    )
    .optional()
}

/// A stored revision column (`NULL` → `None`).
fn revision_col(row: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<Revision>> {
    row.get::<_, Option<String>>(i)?
        .map(|s| {
            s.parse().map_err(|e: String| {
                rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, e.into())
            })
        })
        .transpose()
}

/// Point a snapshot at VCS revision `revision` and flip every file ref
/// pinned to it to that exact version.
fn stamp_revision_tx(
    conn: &rusqlite::Connection,
    snapshot_id: i64,
    revision: &Revision,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE snapshot SET revision = ?1 WHERE id = ?2",
        params![revision.to_string(), snapshot_id],
    )?;
    if let Revision::Vcs { rev, .. } = revision {
        for table in ["effort_file", "page_ref"] {
            conn.execute(
                &format!(
                    "UPDATE {table} SET closest_vcs_rev = ?1, vcs_rev_exact = 1
                      WHERE local_snapshot_id = ?2"
                ),
                params![rev, snapshot_id],
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_op_tx(
    conn: &rusqlite::Connection,
    stream_id: StreamId,
    snapshot_id: i64,
    parent: Option<i64>,
    trigger: SnapshotTrigger,
    anchors: (Option<ThreadId>, Option<i64>, Option<EffortId>),
    elapsed_ms: u64,
    budget_ms: Option<u64>,
    file_count: u32,
) -> rusqlite::Result<i64> {
    let over = budget_ms.is_some_and(|b| elapsed_ms > b);
    conn.execute(
        "INSERT INTO snapshot_op
           (stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id, effort_id,
            at, elapsed_ms, budget_ms, over_budget, file_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            stream_id.value(),
            snapshot_id,
            parent,
            trigger.as_db_str(),
            anchors.0.map(|t| t.value()),
            anchors.1,
            anchors.2.map(|e| e.value()),
            ts_to_string(Timestamp::now()),
            elapsed_ms as i64,
            budget_ms.map(|b| b as i64),
            over as i64,
            file_count as i64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn record_take_tx(
    tx: &rusqlite::Connection,
    vocabulary: &Vocabulary,
    take: &TakeRecord,
) -> Result<Option<TakeOutcome>, DomainError> {
    use crate::database::map_sql_err;
    let parent = current_snapshot_tx(tx, take.stream_id).map_err(map_sql_err)?;
    let (snapshot_id, unchanged, file_count) = if take.rows.is_empty() {
        match parent {
            None => return Ok(None),
            Some(p) => (p, true, 0u32),
        }
    } else {
        tx.execute(
            "INSERT INTO snapshot (stream_id, created_at, branch, revision)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                take.stream_id.value(),
                ts_to_string(Timestamp::now()),
                take.branch,
                take.revision.as_ref().map(Revision::to_string),
            ],
        )
        .map_err(map_sql_err)?;
        let sid = tx.last_insert_rowid();
        let rows: Vec<FileSnapshot> = take
            .rows
            .iter()
            .cloned()
            .map(|mut r| {
                r.snapshot_id = Some(sid);
                r
            })
            .collect();
        insert_rows_tx(tx, &rows).map_err(map_sql_err)?;
        let hash = manifest_hash(&tree_at_conn(tx, sid).map_err(map_sql_err)?);
        let parent_hash: Option<String> = match parent {
            Some(p) => tx
                .query_row(
                    "SELECT tree_hash FROM snapshot WHERE id = ?1",
                    params![p],
                    |r| r.get(0),
                )
                .optional()
                .map_err(map_sql_err)?
                .flatten(),
            None => None,
        };
        match parent {
            // Same files as the parent (e.g. a change and its revert
            // landing in one take): no new snapshot. Its rows go with it.
            Some(p) if parent_hash.as_deref() == Some(hash.as_str()) => {
                tx.execute("DELETE FROM snapshot WHERE id = ?1", params![sid])
                    .map_err(map_sql_err)?;
                (p, true, 0)
            }
            _ => {
                tx.execute(
                    "UPDATE snapshot SET tree_hash = ?2 WHERE id = ?1",
                    params![sid, hash],
                )
                .map_err(map_sql_err)?;
                (sid, false, rows.len() as u32)
            }
        }
    };
    let op_seq = insert_op_tx(
        tx,
        take.stream_id,
        snapshot_id,
        parent,
        take.trigger,
        (take.thread_id, take.turn_id, take.effort_id),
        take.elapsed_ms,
        take.budget_ms,
        file_count,
    )
    .map_err(map_sql_err)?;
    if let Some(turn) = take.turn_id {
        // The turn ended at this snapshot (P2.3).
        tx.execute(
            "UPDATE agent_turn SET snapshot_id = ?2 WHERE id = ?1",
            params![turn, snapshot_id],
        )
        .map_err(map_sql_err)?;
    }
    let over_budget = take.budget_ms.is_some_and(|b| take.elapsed_ms > b);
    let stream = stream_ref(take.stream_id);
    let env = Envelope::typed::<SnapshotTaken>(
        take.source.clone(),
        &SnapshotTakenV2 {
            stream: stream.clone(),
            snapshot: snapshot_ref(snapshot_id),
            parent: parent.map(snapshot_ref),
            trigger: take.trigger,
            unchanged,
            file_count,
            elapsed_ms: take.elapsed_ms,
            budget_ms: take.budget_ms,
            over_budget,
        },
    )
    .with_anchors(Anchors {
        stream_id: Some(take.stream_id),
        thread_id: take.thread_id,
        effort_id: take.effort_id,
        turn_id: take.turn_id,
        snapshot_id: Some(snapshot_id),
        ..Anchors::default()
    })
    .with_subject([stream, snapshot_ref(snapshot_id)]);
    append_tx(tx, vocabulary, &env)?;
    Ok(Some(TakeOutcome {
        op_seq,
        snapshot_id,
        parent_snapshot_id: parent,
        unchanged,
        file_count,
        over_budget,
    }))
}

fn record_head_moved_tx(
    tx: &rusqlite::Connection,
    vocabulary: &Vocabulary,
    stream_id: StreamId,
    expected: i64,
    revision: &Revision,
    source: &str,
) -> Result<Option<TakeOutcome>, DomainError> {
    use crate::database::map_sql_err;
    let sid = expected;
    if current_snapshot_tx(tx, stream_id).map_err(map_sql_err)? != Some(sid) {
        return Ok(None);
    }
    let from: Option<String> = tx
        .query_row(
            "SELECT revision FROM snapshot WHERE id = ?1",
            params![sid],
            |r| r.get(0),
        )
        .map_err(map_sql_err)?;
    if from.as_deref() == Some(revision.to_string().as_str()) {
        return Ok(None);
    }
    stamp_revision_tx(tx, sid, revision).map_err(map_sql_err)?;
    let op_seq = insert_op_tx(
        tx,
        stream_id,
        sid,
        Some(sid),
        SnapshotTrigger::HeadMoved,
        (None, None, None),
        0,
        None,
        0,
    )
    .map_err(map_sql_err)?;
    let stream = stream_ref(stream_id);
    let env = Envelope::typed::<VcsHeadMoved>(
        source.to_string(),
        &VcsHeadMovedV1 {
            stream: stream.clone(),
            snapshot: snapshot_ref(sid),
            from: from
                .and_then(|f| f.parse::<Revision>().ok())
                .and_then(|f| f.vcs_rev().map(commit_ref)),
            to: commit_ref(revision.vcs_rev().unwrap_or_default()),
        },
    )
    .with_anchors(Anchors {
        stream_id: Some(stream_id),
        snapshot_id: Some(sid),
        ..Anchors::default()
    })
    .with_subject([
        stream,
        snapshot_ref(sid),
        commit_ref(revision.vcs_rev().unwrap_or_default()),
    ]);
    append_tx(tx, vocabulary, &env)?;
    Ok(Some(TakeOutcome {
        op_seq,
        snapshot_id: sid,
        parent_snapshot_id: Some(sid),
        unchanged: true,
        file_count: 0,
        over_budget: false,
    }))
}

fn row_to_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotOp> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let trigger: String = row.get(4)?;
    let at: String = row.get(8)?;
    Ok(SnapshotOp {
        seq: row.get(0)?,
        stream_id: StreamId::new(row.get(1)?),
        snapshot_id: row.get(2)?,
        parent_snapshot_id: row.get(3)?,
        trigger: SnapshotTrigger::from_db_str(&trigger).ok_or_else(|| {
            conv(DomainError::Invalid(format!(
                "unknown snapshot trigger `{trigger}`"
            )))
        })?,
        thread_id: row.get::<_, Option<i64>>(5)?.map(ThreadId::new),
        turn_id: row.get(6)?,
        effort_id: row.get::<_, Option<i64>>(7)?.map(EffortId::new),
        at: string_to_ts(&at).map_err(conv)?,
        elapsed_ms: row.get(9)?,
        budget_ms: row.get(10)?,
        over_budget: row.get::<_, i64>(11)? != 0,
        file_count: row.get(12)?,
    })
}

/// One snapshot take, as [`SqliteSnapshotStore::record_take`] writes it.
#[derive(Debug, Clone)]
pub struct TakeRecord {
    pub stream_id: StreamId,
    /// The file rows the take captured (`snapshot_id` is filled in).
    /// Empty when nothing changed.
    pub rows: Vec<FileSnapshot>,
    pub trigger: SnapshotTrigger,
    pub thread_id: Option<ThreadId>,
    /// `agent_turn.id`, when the take belongs to a turn.
    pub turn_id: Option<i64>,
    pub effort_id: Option<EffortId>,
    /// The branch the workspace was on (recorded clean or dirty).
    pub branch: Option<String>,
    /// The head revision when the workspace was clean at the take.
    pub revision: Option<Revision>,
    pub elapsed_ms: u64,
    pub budget_ms: Option<u64>,
    /// The envelope `source` of `snapshot.taken` (`system:snapshot_capture`).
    pub source: String,
}

/// What a take recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeOutcome {
    /// The `snapshot_op.seq` of this take.
    pub op_seq: i64,
    /// The snapshot the worktree is at after the take.
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    /// No new snapshot: nothing changed (or the tree equals the parent's).
    pub unchanged: bool,
    pub file_count: u32,
    pub over_budget: bool,
}

/// One row of the operation log (`snapshot_op`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SnapshotOp {
    pub seq: i64,
    pub stream_id: StreamId,
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    pub trigger: SnapshotTrigger,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<i64>,
    pub effort_id: Option<EffortId>,
    pub at: Timestamp,
    pub elapsed_ms: i64,
    pub budget_ms: Option<i64>,
    pub over_budget: bool,
    pub file_count: i64,
}

#[derive(Clone)]
pub struct SqliteSnapshotStore {
    db: Database,
    content_hasher: Option<ContentHasher>,
    /// Validates the `snapshot.taken` / `vcs.head.moved` envelopes.
    vocabulary: VocabularyHandle,
}

impl SqliteSnapshotStore {
    pub fn new(db: Database) -> Self {
        Self::with_vocabulary(db, VocabularyHandle::core())
    }

    pub fn with_vocabulary(db: Database, vocabulary: VocabularyHandle) -> Self {
        Self {
            db,
            content_hasher: None,
            vocabulary,
        }
    }

    /// Record one take — the only production write path for snapshots
    /// (P2.2, tsk424). In ONE transaction: the new `snapshot` row (with
    /// its branch, clean-tree commit and `tree_hash`), its file rows, the
    /// `snapshot_op` row and the `snapshot.taken` event. When `rows` is
    /// empty, or the resulting tree equals the parent's (`tree_hash`), no
    /// snapshot is created: the op points at the parent and the event
    /// says `unchanged`. `Ok(None)` only when the stream has no snapshot
    /// at all yet and nothing to record.
    pub async fn record_take(&self, take: TakeRecord) -> Result<Option<TakeOutcome>, DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| record_take_tx(tx, &vocabulary.current(), &take))
            .await
    }

    /// HEAD moved while the worktree was clean: re-stamp `snapshot_id` —
    /// the snapshot the caller saw the clean tree at — with `sha` (and
    /// every exact-pin file ref on it), record a `head_moved` op and
    /// `vcs.head.moved`, in one transaction. `Ok(None)` when the stream's
    /// current snapshot is no longer `snapshot_id` (a take landed since:
    /// stamping it would claim a dirty tree is the commit), or it already
    /// points at `sha`.
    pub async fn record_head_moved(
        &self,
        stream_id: StreamId,
        snapshot_id: i64,
        revision: Revision,
        source: String,
    ) -> Result<Option<TakeOutcome>, DomainError> {
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                record_head_moved_tx(
                    tx,
                    &vocabulary.current(),
                    stream_id,
                    snapshot_id,
                    &revision,
                    &source,
                )
            })
            .await
    }

    /// The stream's operation log, newest first.
    pub async fn list_ops(
        &self,
        stream_id: StreamId,
        limit: usize,
    ) -> Result<Vec<SnapshotOp>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id,
                            turn_id, effort_id, at, elapsed_ms, budget_ms, over_budget, file_count
                       FROM snapshot_op WHERE stream_id = ?1
                      ORDER BY seq DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![stream_id.value(), limit as i64], row_to_op)?;
                rows.collect()
            })
            .await
    }

    /// The op that CREATED `snapshot_id` (its first op), which carries the
    /// snapshot's parent and trigger.
    pub async fn op_for_snapshot(
        &self,
        snapshot_id: i64,
    ) -> Result<Option<SnapshotOp>, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id,
                            turn_id, effort_id, at, elapsed_ms, budget_ms, over_budget, file_count
                       FROM snapshot_op WHERE snapshot_id = ?1 ORDER BY seq ASC LIMIT 1",
                    params![snapshot_id],
                    row_to_op,
                )
                .optional()
            })
            .await
    }

    /// Supply the lazy git-content hasher (the app wires one over the git
    /// odb). Without one, an un-hashed git entry compares by `git:<oid>`
    /// only — conservative: the same bytes on the other side read as
    /// changed, never the reverse.
    pub fn with_content_hasher(mut self, hasher: ContentHasher) -> Self {
        self.content_hasher = Some(hasher);
        self
    }

    pub async fn capture(&self, snap: FileSnapshot) -> Result<i64, DomainError> {
        Ok(self.capture_batch(vec![snap]).await?[0])
    }

    /// **Fixture seeding** (tests): insert N `file_snapshot` rows in one
    /// transaction, returning their ids. Production takes go through
    /// [`Self::record_take`], which also writes the snapshot row, its op
    /// and `snapshot.taken` — rows written here have no op.
    pub async fn capture_batch(&self, snaps: Vec<FileSnapshot>) -> Result<Vec<i64>, DomainError> {
        if snaps.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .transaction(move |tx| insert_rows_tx(tx, &snaps).map_err(crate::database::map_sql_err))
            .await
    }

    /// **Fixture seeding** (tests): insert a bare `snapshot` row. A
    /// production take uses [`Self::record_take`].
    pub async fn create_snapshot(&self, stream_id: StreamId) -> Result<i64, DomainError> {
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO snapshot (stream_id, created_at) VALUES (?1, ?2)",
                    params![stream_id.value(), now],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }

    /// The VCS revision a snapshot's tree equals, if recorded.
    pub async fn get_snapshot_revision(
        &self,
        snapshot_id: i64,
    ) -> Result<Option<Revision>, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT revision FROM snapshot WHERE id = ?1",
                    params![snapshot_id],
                    |row| revision_col(row, 0),
                )
                .optional()
                .map(|opt| opt.flatten())
            })
            .await
    }

    /// **Fixture seeding** (tests): pin a snapshot to a revision (with
    /// the exact-pin cascade). Production stamps a new snapshot in
    /// [`Self::record_take`] and a head move in [`Self::record_head_moved`].
    pub async fn set_snapshot_revision(
        &self,
        snapshot_id: i64,
        revision: Revision,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                stamp_revision_tx(tx, snapshot_id, &revision).map_err(crate::database::map_sql_err)
            })
            .await
    }

    /// **Fixture seeding** (tests): set a snapshot's branch. Production
    /// records it in [`Self::record_take`].
    pub async fn set_snapshot_branch(
        &self,
        snapshot_id: i64,
        branch: String,
    ) -> Result<(), DomainError> {
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE snapshot SET branch = ?1 WHERE id = ?2",
                    params![branch, snapshot_id],
                )?;
                Ok(())
            })
            .await
    }

    /// Most recent `snapshot.id` for the stream. Returns `None` when
    /// no snapshots exist yet for the stream.
    pub async fn latest_snapshot_id_for_stream(
        &self,
        stream_id: StreamId,
    ) -> Result<Option<i64>, DomainError> {
        self.db
            .call(move |conn| latest_snapshot_id_for_stream_tx(conn, stream_id))
            .await
    }

    /// The stream's newest snapshot taken at or before `at` — the tree as
    /// it stood then, as far as oxplow saw it. `None` when there's none.
    pub async fn latest_snapshot_at_or_before(
        &self,
        stream_id: StreamId,
        at: Timestamp,
    ) -> Result<Option<i64>, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT id FROM snapshot WHERE stream_id = ?1 AND created_at <= ?2
                     ORDER BY created_at DESC, id DESC LIMIT 1",
                    params![stream_id.value(), ts_to_string(at)],
                    |row| row.get(0),
                )
                .optional()
            })
            .await
    }

    /// Snapshot rows for a stream, newest first.
    pub async fn list_snapshots_for_stream(
        &self,
        stream_id: StreamId,
        limit: usize,
    ) -> Result<Vec<Snapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.id, s.stream_id, s.created_at,
                            (SELECT COUNT(*) FROM file_snapshot f
                             WHERE f.snapshot_id = s.id) AS file_count,
                            s.revision, s.branch, s.tree_hash,
                            op.parent_snapshot_id, op.trigger, COALESCE(op.over_budget, 0)
                     FROM snapshot s
                     LEFT JOIN snapshot_op op ON op.seq = (
                         SELECT MIN(o.seq) FROM snapshot_op o WHERE o.snapshot_id = s.id)
                     WHERE s.stream_id = ?1
                     ORDER BY s.created_at DESC, s.id DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![stream_id.value(), limit as i64], |row| {
                    let id: i64 = row.get(0)?;
                    let stream_id: i64 = row.get(1)?;
                    let created_at: String = row.get(2)?;
                    let file_count: i64 = row.get(3)?;
                    let revision = revision_col(row, 4)?;
                    let branch: Option<String> = row.get(5)?;
                    let tree_hash: Option<String> = row.get(6)?;
                    let parent_snapshot_id: Option<i64> = row.get(7)?;
                    let trigger: Option<String> = row.get(8)?;
                    let over_budget: i64 = row.get(9)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(Snapshot {
                        id,
                        stream_id: StreamId::new(stream_id),
                        created_at: string_to_ts(&created_at).map_err(map_err)?,
                        file_count,
                        revision,
                        branch,
                        tree_hash,
                        parent_snapshot_id,
                        trigger: trigger.as_deref().and_then(SnapshotTrigger::from_db_str),
                        over_budget: over_budget != 0,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Every commit-stamped snapshot, oldest first — the metric fold's ancestry
    /// anchor points (tsk102). A stamped snapshot means "this snapshot IS
    /// exactly that commit's tree" (stamped by the capture layer on a clean
    /// worktree, or re-stamped by the git-refs listener when a commit lands on
    /// an unchanged worktree), so the first same-stream, same-branch stamp
    /// at-or-after a dirty capture names the commit that ABSORBED its work.
    /// One row per commit the stream actually made — small by construction.
    pub async fn commit_stamped_snapshots(&self) -> Result<Vec<StampedSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, branch, substr(revision, instr(revision, ':') + 1), created_at
                       FROM snapshot
                      WHERE revision IS NOT NULL
                      ORDER BY created_at ASC, id ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    let created_at: String = row.get(4)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(StampedSnapshot {
                        id: row.get(0)?,
                        stream_id: row.get(1)?,
                        branch: row.get(2)?,
                        commit: row.get(3)?,
                        created_at: string_to_ts(&created_at).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            // Belt-and-braces: writes are canonical and V67 normalized old
            // rows (tsk107), so the SQL ORDER BY is chronological — but
            // "oldest first" is load-bearing for the resolver's
            // first-stamp-at-or-after search, so the parsed-value sort stays
            // as insurance against any stray non-canonical value.
            .map(|mut rows| {
                rows.sort_by_key(|r| (r.created_at, r.id));
                rows
            })
    }

    /// Created / modified / deleted counts for the rows captured under one
    /// snapshot, each compared with the most recent prior row for the same
    /// `(stream_id, path)` by **content identity** (V96): a row whose bytes
    /// equal its predecessor's is not a change and isn't counted (nor in
    /// `total`). A git row not yet hashed compares as `git:<oid>`, so it can
    /// read as modified against a hashed predecessor — conservative. The
    /// `idx_file_snapshot_stream_path` index covers the prior-row lookup.
    pub async fn stats_for_snapshot(&self, snapshot_id: i64) -> Result<SnapshotStats, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT
                           COALESCE(SUM(kind = 'deleted'), 0),
                           COALESCE(SUM(kind = 'added'), 0),
                           COALESCE(SUM(kind = 'modified'), 0),
                           COUNT(*)
                         FROM ({CLASSIFIED_CHANGES}) WHERE kind IS NOT NULL"
                    ),
                    params![snapshot_id],
                    |row| {
                        Ok(SnapshotStats {
                            deleted: row.get(0)?,
                            created: row.get(1)?,
                            modified: row.get(2)?,
                            total: row.get(3)?,
                        })
                    },
                )
            })
            .await
    }

    /// `SnapshotChangeEntry` rows for one snapshot: every captured row that
    /// is a change against the path's prior row (same classification as
    /// [`Self::stats_for_snapshot`]), labelled `added`/`modified`/`deleted`
    /// so the renderer can feed the same shape into the shared
    /// change-analysis pipeline used by Git commits.
    pub async fn list_changes_for_snapshot(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<SnapshotChangeEntry>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT id, path, storage, prior_id, kind
                       FROM ({CLASSIFIED_CHANGES}) WHERE kind IS NOT NULL
                      ORDER BY path ASC"
                ))?;
                let rows = stmt.query_map(params![snapshot_id], |row| {
                    let storage = SnapshotStorage::from_db_str(&row.get::<_, String>(2)?);
                    Ok(SnapshotChangeEntry {
                        current_file_id: row.get(0)?,
                        path: row.get(1)?,
                        prior_file_id: row.get(3)?,
                        status: row.get(4)?,
                        oversize: storage.is_oversize(),
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Reconstruct the tree as of `snapshot_id`: the most-recent
    /// `file_snapshot` row per path with `snapshot_id <= snapshot_id`
    /// (snapshots are incremental deltas). Deletion tombstones drop the
    /// path. Entries keep storage/address apart from content identity
    /// ([`TreeEntry`]); to compare two trees, run
    /// [`Self::resolve_for_compare`] first (or use [`Self::diff_snapshots`]).
    pub async fn tree_at(&self, snapshot_id: i64) -> Result<SnapshotTree, DomainError> {
        self.db
            .call(move |conn| tree_at_conn(conn, snapshot_id))
            .await
    }

    /// Make two trees comparable: hash (lazily, once, persisted) every
    /// un-hashed git entry that sits opposite an entry with a different
    /// identity — the only case where the OID-vs-xxh3 split could call
    /// equal bytes different. Paths equal by identity, or present on one
    /// side only, never need it, so a clean git baseline is not re-read.
    /// Without a hasher this is a no-op (conservative).
    pub async fn resolve_for_compare(
        &self,
        before: &mut SnapshotTree,
        after: &mut SnapshotTree,
    ) -> Result<(), DomainError> {
        let Some(hasher) = self.content_hasher.clone() else {
            return Ok(());
        };
        let mut oids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (path, a) in after.iter() {
            let Some(b) = before.get(path) else { continue };
            if a.identity() == b.identity() {
                continue;
            }
            for e in [a, b] {
                if e.needs_content_hash() {
                    if let Some(oid) = &e.address {
                        oids.insert(oid.clone());
                    }
                }
            }
        }
        if oids.is_empty() {
            return Ok(());
        }
        let hashed: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
            oids.into_iter()
                .filter_map(|oid| hasher(&oid).map(|h| (oid, h)))
                .collect()
        })
        .await
        .map_err(|e| DomainError::Storage(format!("content hash task: {e}")))?;
        if hashed.is_empty() {
            return Ok(());
        }
        let by_oid: std::collections::HashMap<String, String> = hashed.iter().cloned().collect();
        for tree in [&mut *before, &mut *after] {
            for e in tree.values_mut() {
                if e.needs_content_hash() {
                    if let Some(h) = e.address.as_ref().and_then(|oid| by_oid.get(oid)) {
                        e.content_hash = Some(h.clone());
                    }
                }
            }
        }
        self.set_git_content_hashes(hashed).await
    }

    /// Persist content hashes for git-backed rows, by OID. An OID names
    /// its bytes forever, so every row with that address gets the hash.
    async fn set_git_content_hashes(
        &self,
        hashed: Vec<(String, String)>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "UPDATE file_snapshot SET content_hash = ?2
                          WHERE storage = 'git' AND blob_hash = ?1 AND content_hash IS NULL",
                    )
                    .map_err(crate::database::map_sql_err)?;
                for (oid, hash) in &hashed {
                    stmt.execute(params![oid, hash])
                        .map_err(crate::database::map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Content diff between two snapshots via the shared
    /// [`oxplow_domain::diff_trees`], comparing content identity (never a
    /// storage address). `from = None` ⇒ everything in `to` is added.
    pub async fn diff_snapshots(
        &self,
        from: Option<i64>,
        to: i64,
    ) -> Result<Vec<FileChange>, DomainError> {
        let mut before = match from {
            Some(f) => self.tree_at(f).await?,
            None => SnapshotTree::new(),
        };
        let mut after = self.tree_at(to).await?;
        self.resolve_for_compare(&mut before, &mut after).await?;
        Ok(diff_trees(&identities(&before), &identities(&after)))
    }

    /// `snapshot_id`'s `tree_hash`: two snapshots with the same one hold
    /// the same files. `None` when it has none recorded.
    pub async fn tree_hash(&self, snapshot_id: i64) -> Result<Option<String>, DomainError> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT tree_hash FROM snapshot WHERE id = ?1",
                    params![snapshot_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
                .map(Option::flatten)
            })
            .await
    }

    /// Compute and store `snapshot.tree_hash` for `snapshot_id` from its
    /// reconstructed tree. Returns the hash.
    pub async fn set_tree_hash(&self, snapshot_id: i64) -> Result<String, DomainError> {
        self.db
            .call(move |conn| {
                let hash = manifest_hash(&tree_at_conn(conn, snapshot_id)?);
                conn.execute(
                    "UPDATE snapshot SET tree_hash = ?2 WHERE id = ?1",
                    params![snapshot_id, hash],
                )?;
                Ok(hash)
            })
            .await
    }

    /// For each input snapshot id, return the set of wiki slugs whose
    /// `.md` body changed in that snapshot's file_snapshot rows.
    /// Cheap targeted query for the Local History dashboard's wiki
    /// badges — avoids fetching the full file list per snapshot.
    pub async fn list_wiki_slugs_for_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<(i64, String)>, DomainError> {
        if snapshot_ids.is_empty() {
            return Ok(vec![]);
        }
        self.db
            .call(move |conn| {
                let placeholders: Vec<String> =
                    (1..=snapshot_ids.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT snapshot_id, path FROM file_snapshot \
                     WHERE snapshot_id IN ({}) \
                       AND path LIKE '.oxplow/wiki/%.md'",
                    placeholders.join(",")
                );
                let mut stmt = conn.prepare(&sql)?;
                let params_iter: Vec<&dyn rusqlite::ToSql> = snapshot_ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt.query_map(rusqlite::params_from_iter(params_iter), |row| {
                    let sid: i64 = row.get("snapshot_id")?;
                    let path: String = row.get("path")?;
                    let slug = path
                        .strip_prefix(".oxplow/wiki/")
                        .and_then(|s| s.strip_suffix(".md"))
                        .map(|s| s.to_string())
                        .unwrap_or(path);
                    Ok((sid, slug))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn list_files_for_snapshot(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage,
                            snapshot_id
                     FROM file_snapshot WHERE snapshot_id = ?1 ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![snapshot_id], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The files recorded since `after` (exclusive) up to `to`, each at its
    /// latest row in that span — what an incremental scan would have seen
    /// had it run on every snapshot between (a paced collector's deferred
    /// run). Deletions included, as a snapshot's own rows are.
    pub async fn list_span_files(
        &self,
        after: i64,
        to: i64,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage,
                            snapshot_id
                     FROM (
                        SELECT f.id, f.stream_id, f.path, f.blob_hash, f.size_bytes, f.captured_at,
                               f.storage, f.snapshot_id,
                               ROW_NUMBER() OVER (
                                 PARTITION BY f.path ORDER BY f.snapshot_id DESC, f.id DESC
                               ) AS rn
                        FROM file_snapshot f
                        WHERE f.stream_id = (SELECT stream_id FROM snapshot WHERE id = ?2)
                          AND f.snapshot_id > ?1 AND f.snapshot_id <= ?2
                     ) WHERE rn = 1
                     ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![after, to], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The RECONSTRUCTED tree as-of `snapshot_id`: the latest `file_snapshot`
    /// row per path with `snapshot_id <= ?` (same window as [`Self::tree_at`]),
    /// excluding paths whose latest row is a deletion tombstone. This is the
    /// whole tree's file rows — content-addressable via `blob_hash`/`storage` —
    /// even though the anchor snapshot itself only lists a delta. The baseline
    /// gauge sweep (tsk71) builds its full-tree corpus from this instead of
    /// requiring a fabricated full-tree snapshot.
    pub async fn list_tree_files_at(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let stream_id: Option<i64> = conn
                    .query_row(
                        "SELECT stream_id FROM snapshot WHERE id = ?1",
                        params![snapshot_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(stream_id) = stream_id else {
                    return Ok(Vec::new());
                };
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage,
                            snapshot_id
                     FROM (
                        SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage,
                               snapshot_id,
                               ROW_NUMBER() OVER (
                                 PARTITION BY path ORDER BY snapshot_id DESC, id DESC
                               ) AS rn
                        FROM file_snapshot
                        WHERE stream_id = ?1
                          AND snapshot_id IS NOT NULL
                          AND snapshot_id <= ?2
                     ) WHERE rn = 1 AND storage <> 'deleted'
                     ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![stream_id, snapshot_id], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    pub async fn get(&self, id: i64) -> Result<Option<FileSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE id = ?1",
                )?;
                let mut rows = stmt.query_map(params![id], row_to_snapshot)?;
                rows.next().transpose()
            })
            .await
    }

    pub async fn list_for_stream(
        &self,
        stream_id: StreamId,
        limit: usize,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE stream_id = ?1
                     ORDER BY captured_at DESC, id DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![stream_id.value(), limit as i64], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Most recent row's stat and identity per path **in one stream**. Used
    /// by the startup sweep (when `(size, mtime)` matches, the bytes are
    /// presumed identical and not re-read) and by capture (a re-captured
    /// path whose content equals its latest row writes no new row).
    /// `mtime_ms` is `None` for pre-V15 rows. Another stream's rows are a
    /// different worktree's history and never count here.
    pub async fn latest_stat_per_path(
        &self,
        stream_id: StreamId,
    ) -> Result<std::collections::HashMap<String, LatestStat>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.path, s.blob_hash, s.size_bytes, s.mtime_ms, s.storage, s.content_hash
                     FROM file_snapshot s
                     JOIN (
                       SELECT path, MAX(id) AS max_id
                       FROM file_snapshot WHERE stream_id = ?1 GROUP BY path
                     ) m ON m.path = s.path AND m.max_id = s.id",
                )?;
                let rows = stmt.query_map(params![stream_id.value()], |row| {
                    let path: String = row.get(0)?;
                    let storage = SnapshotStorage::from_db_str(&row.get::<_, String>(4)?);
                    let blob_hash: Option<String> = row.get(1)?;
                    let content_hash: Option<String> =
                        row.get::<_, Option<String>>(5)?.or_else(|| {
                            (storage == SnapshotStorage::Oxplow)
                                .then(|| blob_hash.clone())
                                .flatten()
                        });
                    Ok((
                        path,
                        LatestStat {
                            blob_hash,
                            size_bytes: row.get(2)?,
                            mtime_ms: row.get(3)?,
                            storage,
                            content_hash,
                        },
                    ))
                })?;
                rows.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()
            })
            .await
    }

    /// Distinct non-null `blob_hash` values referenced by any row.
    /// Used by blob GC to decide which on-disk content is still live.
    /// Test-only: rewrite the oldest row for `path` to a given
    /// `captured_at`. Lets cleanup tests construct rows that fall
    /// outside a retention window without time-traveling the clock.
    #[doc(hidden)]
    pub async fn backdate_for_test(self: std::sync::Arc<Self>, path: &str, ts: Timestamp) {
        let path = path.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE file_snapshot SET captured_at = ?1
                     WHERE id = (SELECT MIN(id) FROM file_snapshot WHERE path = ?2)",
                    params![ts_to_string(ts), path],
                )?;
                Ok(())
            })
            .await
            .expect("backdate_for_test snapshot update");
    }

    /// Delete snapshot rows whose `captured_at` is older than
    /// `cutoff`, except the most-recent row per path (so every
    /// file keeps at least one history entry no matter how old).
    /// Returns the number of rows deleted.
    /// The blob hashes the daily cleanup must KEEP on disk: every row inside
    /// the retention window, plus each `(stream, path)`'s newest row at ANY
    /// age — so every worktree's current tree stays viewable/rollbackable
    /// forever. Every other hash is content the cleanup may expire.
    ///
    /// The ROWS are never deleted (tsk105): retention's job is bounding the
    /// on-disk file copies, and the records are a different thing — durable
    /// facts other subsystems replay (the per-path metric fold derives each
    /// capture's RESTATED SET from them, and the ancestry anchors ride the
    /// parent `snapshot` rows). Rows weigh bytes; blobs weigh megabytes.
    /// Deleting rows to save disk was aiming at the wrong mass, and it rotted
    /// the fold's inputs out from under durable captures.
    pub async fn retained_blob_hashes(
        &self,
        cutoff: Timestamp,
    ) -> Result<std::collections::HashSet<String>, DomainError> {
        let cutoff_str = ts_to_string(cutoff);
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT blob_hash FROM file_snapshot
                      WHERE blob_hash IS NOT NULL
                        AND (captured_at >= ?1
                             OR id IN (SELECT MAX(id) FROM file_snapshot
                                        GROUP BY stream_id, path))",
                )?;
                let rows = stmt.query_map(params![cutoff_str], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()
            })
            .await
    }

    pub async fn list_for_path(&self, path: &str) -> Result<Vec<FileSnapshot>, DomainError> {
        let path = path.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE path = ?1 ORDER BY captured_at DESC, id DESC",
                )?;
                let rows = stmt.query_map(params![path], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Return a readable content handle for `path` as it existed at
    /// `snapshot_id`: the most-recent `file_snapshot` row for that path
    /// whose `snapshot_id <= given`. Returns `Ok(None)` when the file
    /// was absent, deleted, or oversize (no readable bytes) at that
    /// point. The [`SnapshotContentRef::storage`] tells the caller where
    /// to read the bytes from (oxplow blob store vs git odb).
    pub async fn content_ref_for_path(
        &self,
        snapshot_id: i64,
        path: &str,
    ) -> Result<Option<SnapshotContentRef>, DomainError> {
        let path = path.to_string();
        self.db
            .call(move |conn| {
                let stream_id: Option<i64> = conn
                    .query_row(
                        "SELECT stream_id FROM snapshot WHERE id = ?1",
                        params![snapshot_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(stream_id) = stream_id else {
                    return Ok(None);
                };
                // Latest file_snapshot for this path at or before snapshot_id.
                // Only oxplow/git rows carry readable bytes; oversize and
                // deletion tombstones resolve to None.
                conn.query_row(
                    "SELECT blob_hash, storage FROM file_snapshot
                     WHERE stream_id = ?1
                       AND path = ?2
                       AND snapshot_id IS NOT NULL
                       AND snapshot_id <= ?3
                     ORDER BY snapshot_id DESC, id DESC
                     LIMIT 1",
                    params![stream_id, path, snapshot_id],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()
                .map(|opt| {
                    opt.and_then(|(hash, storage)| {
                        let storage = SnapshotStorage::from_db_str(&storage);
                        match (storage.has_bytes(), hash) {
                            (true, Some(hash)) => Some(SnapshotContentRef { storage, hash }),
                            _ => None,
                        }
                    })
                })
            })
            .await
    }
}

/// Most recent `snapshot.id` for the stream, read on the caller's
/// connection; `None` when the stream has no snapshot yet.
pub fn latest_snapshot_id_for_stream_tx(
    conn: &rusqlite::Connection,
    stream_id: StreamId,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM snapshot WHERE stream_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT 1",
        params![stream_id.value()],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ChangeStatus;

    #[test]
    fn timestamps_serialize_in_the_canonical_fixed_width_form() {
        // tsk107. Every `ORDER BY … _at` in this store compares these strings
        // lexicographically, and the trimmed RFC-3339 the `time` crate emits
        // inverts same-second neighbors ("…20.5Z" > "…20.51Z"). A whole-second
        // value is the deterministic trap: trimmed form has NO fraction.
        assert_eq!(
            ts_to_string(Timestamp::from_unix_ms(1_000)),
            "1970-01-01T00:00:01.000000Z",
            "fixed 6-digit fraction, always 27 chars"
        );
        assert_eq!(ts_to_string(Timestamp::from_unix_ms(1_500)).len(), 27);
    }

    /// Seed a stream + snapshots + file_snapshot rows for diff tests.
    /// `rows`: (snapshot_id, path, blob_hash, oversize).
    async fn seed_snapshots(db: &Database, rows: &[(i64, &str, Option<&str>, bool)]) {
        let snaps: std::collections::BTreeSet<i64> = rows.iter().map(|(s, ..)| *s).collect();
        let rows: Vec<(i64, String, Option<String>, bool)> = rows
            .iter()
            .map(|(s, p, h, o)| (*s, p.to_string(), h.map(|s| s.to_string()), *o))
            .collect();
        let db = db.clone();
        db.call(move |conn| {
            conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
            conn.execute(
                "INSERT INTO streams
                       (id, kind, title, branch, branch_ref, branch_source,
                        worktree_path, created_at, updated_at)
                     VALUES (1,'primary','s','main','refs/heads/main','origin',
                             '/tmp','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                [],
            )?;
            for sid in &snaps {
                conn.execute(
                    "INSERT INTO snapshot (id, stream_id, created_at)
                         VALUES (?1, 1, '2026-01-01T00:00:00Z')",
                    params![sid],
                )?;
            }
            for (sid, path, hash, oversize) in &rows {
                // Map the legacy (oversize bool, hash) test tuple onto the
                // explicit storage class: oversize → oversize; no-hash →
                // deletion tombstone; otherwise oxplow.
                let storage = if *oversize {
                    "oversize"
                } else if hash.is_none() {
                    "deleted"
                } else {
                    "oxplow"
                };
                conn.execute(
                    "INSERT INTO file_snapshot
                           (stream_id, path, blob_hash, size_bytes, captured_at,
                            storage, snapshot_id, mtime_ms)
                         VALUES (1, ?1, ?2, 10, '2026-01-01T00:00:00Z', ?3, ?4, 1)",
                    params![path, hash, storage, sid],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn content_ref_for_path_returns_latest_at_or_before_snapshot() {
        let db = Database::in_memory();
        let store = SqliteSnapshotStore::new(db.clone());
        // snap 1: a.txt=hA, b.txt=hB
        // snap 2: a.txt=hA2 (modified), b.txt deleted (NULL hash, oversize=0)
        // snap 3: c.txt=hC (new)
        seed_snapshots(
            &db,
            &[
                (1, "a.txt", Some("hA"), false),
                (1, "b.txt", Some("hB"), false),
                (2, "a.txt", Some("hA2"), false),
                (2, "b.txt", None, false), // deletion row
                (3, "c.txt", Some("hC"), false),
            ],
        )
        .await;

        // At snap 1: a.txt has hA, b.txt has hB, c.txt absent
        assert_eq!(
            store
                .content_ref_for_path(1, "a.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            Some("hA".into())
        );
        assert_eq!(
            store
                .content_ref_for_path(1, "b.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            Some("hB".into())
        );
        assert_eq!(
            store
                .content_ref_for_path(1, "c.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            None
        );

        // At snap 2: a.txt updated, b.txt deleted, c.txt still absent
        assert_eq!(
            store
                .content_ref_for_path(2, "a.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            Some("hA2".into())
        );
        assert_eq!(
            store
                .content_ref_for_path(2, "b.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            None
        );
        assert_eq!(
            store
                .content_ref_for_path(2, "c.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            None
        );

        // At snap 3: c.txt now present, a.txt still hA2 from snap 2
        assert_eq!(
            store
                .content_ref_for_path(3, "a.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            Some("hA2".into())
        );
        assert_eq!(
            store
                .content_ref_for_path(3, "c.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            Some("hC".into())
        );
    }

    #[tokio::test]
    async fn content_ref_for_path_returns_none_for_oversize() {
        let db = Database::in_memory();
        let store = SqliteSnapshotStore::new(db.clone());
        seed_snapshots(&db, &[(1, "big.bin", None, true)]).await;
        assert_eq!(
            store
                .content_ref_for_path(1, "big.bin")
                .await
                .unwrap()
                .map(|r| r.hash),
            None
        );
    }

    #[tokio::test]
    async fn content_ref_for_path_returns_none_for_unknown_snapshot() {
        let db = Database::in_memory();
        let store = SqliteSnapshotStore::new(db);
        assert_eq!(
            store
                .content_ref_for_path(999, "a.txt")
                .await
                .unwrap()
                .map(|r| r.hash),
            None
        );
    }

    #[tokio::test]
    async fn tree_at_reconstructs_latest_row_per_path() {
        let db = Database::in_memory();
        let store = SqliteSnapshotStore::new(db.clone());
        seed_snapshots(
            &db,
            &[
                (1, "a.txt", Some("hA"), false),
                (1, "b.txt", Some("hB"), false),
                (2, "a.txt", Some("hA2"), false), // a modified at snap 2
                (2, "c.txt", Some("hC"), false),  // c added at snap 2
            ],
        )
        .await;

        let t1 = store.tree_at(1).await.unwrap();
        assert_eq!(
            t1.get("a.txt").map(TreeEntry::identity).as_deref(),
            Some("hA")
        );
        assert_eq!(
            t1.get("b.txt").map(TreeEntry::identity).as_deref(),
            Some("hB")
        );
        assert!(!t1.contains_key("c.txt"));

        let t2 = store.tree_at(2).await.unwrap();
        // b carries forward (no row at snap 2); a is the newer hash.
        assert_eq!(
            t2.get("a.txt").map(TreeEntry::identity).as_deref(),
            Some("hA2")
        );
        assert_eq!(
            t2.get("b.txt").map(TreeEntry::identity).as_deref(),
            Some("hB")
        );
        assert_eq!(
            t2.get("c.txt").map(TreeEntry::identity).as_deref(),
            Some("hC")
        );
    }

    #[tokio::test]
    async fn diff_snapshots_classifies_and_omits_noops() {
        let db = Database::in_memory();
        let store = SqliteSnapshotStore::new(db.clone());
        seed_snapshots(
            &db,
            &[
                (1, "a.txt", Some("hA"), false),
                (1, "b.txt", Some("hB"), false),
                (2, "a.txt", Some("hA2"), false), // modified
                (2, "c.txt", Some("hC"), false),  // added
                (3, "a.txt", Some("hA2"), false), // no-op rewrite (same hash)
                (4, "b.txt", None, false),        // deletion row
            ],
        )
        .await;

        let d12 = store.diff_snapshots(Some(1), 2).await.unwrap();
        assert_eq!(
            d12,
            vec![
                FileChange {
                    path: "a.txt".into(),
                    status: ChangeStatus::Modified
                },
                FileChange {
                    path: "c.txt".into(),
                    status: ChangeStatus::Added
                },
            ]
        );

        // snap 2 -> 3 is a byte-identical rewrite of a.txt: no change.
        assert!(store.diff_snapshots(Some(2), 3).await.unwrap().is_empty());

        // b.txt deleted by snap 4.
        let d14 = store.diff_snapshots(Some(1), 4).await.unwrap();
        assert!(d14.contains(&FileChange {
            path: "b.txt".into(),
            status: ChangeStatus::Deleted
        }));

        // from = None ⇒ everything in `to` is added.
        let d_none = store.diff_snapshots(None, 1).await.unwrap();
        assert_eq!(d_none.len(), 2);
        assert!(d_none.iter().all(|c| c.status == ChangeStatus::Added));
    }

    /// One typed row for [`seed_typed`].
    struct Seed<'a> {
        snapshot: i64,
        stream: i64,
        path: &'a str,
        storage: &'a str,
        blob: Option<&'a str>,
        content: Option<&'a str>,
    }

    /// `(snapshot, stream, path, storage, blob_hash, content_hash)`.
    fn seed<'a>(
        snapshot: i64,
        stream: i64,
        path: &'a str,
        storage: &'a str,
        blob: Option<&'a str>,
        content: Option<&'a str>,
    ) -> Seed<'a> {
        Seed {
            snapshot,
            stream,
            path,
            storage,
            blob,
            content,
        }
    }

    /// Insert typed snapshot rows (streams and snapshot rows as needed).
    async fn seed_typed(db: &Database, rows: &[Seed<'_>]) {
        let sql: Vec<(i64, i64, [Option<String>; 4])> = rows
            .iter()
            .map(|r| {
                (
                    r.snapshot,
                    r.stream,
                    [
                        Some(r.path.to_string()),
                        Some(r.storage.to_string()),
                        r.blob.map(str::to_string),
                        r.content.map(str::to_string),
                    ],
                )
            })
            .collect();
        db.call(move |conn| {
            for (sn, st, [path, storage, blob, content]) in &sql {
                conn.execute(
                    "INSERT OR IGNORE INTO streams
                       (id, kind, title, branch, branch_ref, branch_source,
                        worktree_path, created_at, updated_at)
                     VALUES (?1,'worktree','s','b','refs/heads/b','origin',
                             '/tmp','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                    params![st],
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO snapshot (id, stream_id, created_at)
                     VALUES (?1, ?2, '2026-01-01T00:00:00Z')",
                    params![sn, st],
                )?;
                conn.execute(
                    "INSERT INTO file_snapshot
                       (stream_id, path, blob_hash, size_bytes, captured_at,
                        storage, snapshot_id, mtime_ms, content_hash)
                     VALUES (?1, ?2, ?3, 10, '2026-01-01T00:00:00Z', ?4, ?5, 1, ?6)",
                    params![st, path, blob, storage, sn, content],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    fn oid(c: char) -> String {
        std::iter::repeat_n(c, 40).collect()
    }

    #[tokio::test]
    async fn the_same_bytes_as_git_and_oxplow_rows_diff_as_unchanged() {
        // tsk423. Snapshot 1 recorded a.txt from the git odb (address = OID,
        // content not hashed yet); snapshot 2 re-captured the same bytes
        // into the blob store (address = content = xxh3 "x1").
        let db = Database::in_memory();
        let o = oid('a');
        seed_typed(
            &db,
            &[
                seed(1, 1, "a.txt", "git", Some(&o), None),
                seed(1, 1, "b.txt", "git", Some(&oid('b')), None),
                seed(2, 1, "a.txt", "oxplow", Some("x1"), Some("x1")),
            ],
        )
        .await;
        // Without a hasher the comparison is conservative: modified.
        let bare = SqliteSnapshotStore::new(db.clone());
        assert_eq!(
            bare.diff_snapshots(Some(1), 2).await.unwrap(),
            vec![FileChange {
                path: "a.txt".into(),
                status: ChangeStatus::Modified
            }]
        );
        // With the hasher (OID → xxh3 of the blob) they are the same bytes.
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let o2 = o.clone();
        let store = SqliteSnapshotStore::new(db.clone()).with_content_hasher(std::sync::Arc::new(
            move |q: &str| {
                seen.lock().unwrap().push(q.to_string());
                (q == o2).then(|| "x1".to_string())
            },
        ));
        assert!(store.diff_snapshots(Some(1), 2).await.unwrap().is_empty());
        // Only the path that looked different was hashed (b.txt was not read).
        assert_eq!(*calls.lock().unwrap(), vec![o.clone()]);
        // …and the hash is persisted: the next compare needs no hasher.
        assert!(bare.diff_snapshots(Some(1), 2).await.unwrap().is_empty());
        let t1 = bare.tree_at(1).await.unwrap();
        assert_eq!(t1["a.txt"].content_hash.as_deref(), Some("x1"));
        assert_eq!(t1["a.txt"].address.as_deref(), Some(o.as_str()));
        assert_eq!(t1["a.txt"].storage, SnapshotStorage::Git);
        assert_eq!(t1["b.txt"].identity(), format!("git:{}", oid('b')));
    }

    #[tokio::test]
    async fn stats_and_changes_ignore_rows_whose_content_did_not_change() {
        let db = Database::in_memory();
        seed_typed(
            &db,
            &[
                seed(1, 1, "same.txt", "oxplow", Some("s1"), Some("s1")),
                seed(1, 1, "edit.txt", "oxplow", Some("e1"), Some("e1")),
                seed(1, 1, "gone.txt", "oxplow", Some("g1"), Some("g1")),
                seed(2, 1, "same.txt", "oxplow", Some("s1"), Some("s1")), // touched, same bytes
                seed(2, 1, "edit.txt", "oxplow", Some("e2"), Some("e2")),
                seed(2, 1, "gone.txt", "deleted", None, None),
                seed(2, 1, "new.txt", "oxplow", Some("n1"), Some("n1")),
            ],
        )
        .await;
        let store = SqliteSnapshotStore::new(db);
        assert_eq!(
            store.stats_for_snapshot(2).await.unwrap(),
            SnapshotStats {
                created: 1,
                modified: 1,
                deleted: 1,
                total: 3
            }
        );
        let changes: Vec<(String, String)> = store
            .list_changes_for_snapshot(2)
            .await
            .unwrap()
            .into_iter()
            .map(|c| (c.path, c.status))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("edit.txt".into(), "modified".into()),
                ("gone.txt".into(), "deleted".into()),
                ("new.txt".into(), "added".into()),
            ]
        );
    }

    #[tokio::test]
    async fn latest_stat_per_path_is_scoped_to_one_stream() {
        let db = Database::in_memory();
        seed_typed(
            &db,
            &[
                seed(1, 1, "a.txt", "oxplow", Some("a1"), Some("a1")),
                seed(2, 2, "a.txt", "oxplow", Some("a2"), Some("a2")),
                seed(3, 2, "only2.txt", "git", Some(&oid('c')), None),
            ],
        )
        .await;
        let store = SqliteSnapshotStore::new(db);
        let s1 = store.latest_stat_per_path(StreamId::new(1)).await.unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1["a.txt"].content_hash.as_deref(), Some("a1"));
        let s2 = store.latest_stat_per_path(StreamId::new(2)).await.unwrap();
        assert_eq!(s2["a.txt"].content_hash.as_deref(), Some("a2"));
        assert_eq!(s2["only2.txt"].storage, SnapshotStorage::Git);
        assert_eq!(s2["only2.txt"].content_hash, None);
    }

    #[tokio::test]
    async fn tree_hash_is_the_manifest_hash_and_equal_trees_share_it() {
        let db = Database::in_memory();
        seed_typed(
            &db,
            &[
                seed(1, 1, "a.txt", "oxplow", Some("a1"), Some("a1")),
                seed(1, 1, "b.txt", "oxplow", Some("b1"), Some("b1")),
                seed(2, 1, "a.txt", "oxplow", Some("a2"), Some("a2")),
                seed(3, 1, "a.txt", "oxplow", Some("a1"), Some("a1")), // reverted
            ],
        )
        .await;
        let store = SqliteSnapshotStore::new(db);
        let h1 = store.set_tree_hash(1).await.unwrap();
        let h2 = store.set_tree_hash(2).await.unwrap();
        let h3 = store.set_tree_hash(3).await.unwrap();
        assert_eq!(
            h1,
            crate::snapshot_tree::manifest_hash(&store.tree_at(1).await.unwrap())
        );
        assert_ne!(h1, h2);
        assert_eq!(h1, h3, "a revert restores the tree identity");
        let listed = store
            .list_snapshots_for_stream(StreamId::new(1), 10)
            .await
            .unwrap();
        assert!(listed.iter().all(|s| s.tree_hash.is_some()));
    }

    #[tokio::test]
    async fn capture_batch_records_content_hash_for_oxplow_rows() {
        let db = Database::in_memory();
        seed_typed(
            &db,
            &[seed(1, 1, "seed.txt", "oxplow", Some("s"), Some("s"))],
        )
        .await;
        let store = SqliteSnapshotStore::new(db);
        let row = |path: &str, storage, blob: Option<&str>| FileSnapshot {
            id: 0,
            stream_id: StreamId::new(1),
            path: path.into(),
            blob_hash: blob.map(str::to_string),
            size_bytes: 1,
            captured_at: Timestamp::now(),
            storage,
            snapshot_id: Some(1),
            mtime_ms: Some(1),
            content_hash: None,
        };
        store
            .capture_batch(vec![
                row("o.txt", SnapshotStorage::Oxplow, Some("xo")),
                row("g.txt", SnapshotStorage::Git, Some(&oid('d'))),
            ])
            .await
            .unwrap();
        let t = store.tree_at(1).await.unwrap();
        assert_eq!(t["o.txt"].content_hash.as_deref(), Some("xo"));
        assert_eq!(t["g.txt"].content_hash, None, "git rows are hashed lazily");
    }

    fn take(stream: i64, rows: Vec<(&str, &str)>, trigger: SnapshotTrigger) -> TakeRecord {
        TakeRecord {
            stream_id: StreamId::new(stream),
            rows: rows
                .into_iter()
                .map(|(path, hash)| FileSnapshot {
                    id: 0,
                    stream_id: StreamId::new(stream),
                    path: path.into(),
                    blob_hash: Some(hash.into()),
                    size_bytes: 1,
                    captured_at: Timestamp::now(),
                    storage: SnapshotStorage::Oxplow,
                    snapshot_id: None,
                    mtime_ms: Some(1),
                    content_hash: None,
                })
                .collect(),
            trigger,
            thread_id: None,
            turn_id: None,
            effort_id: None,
            branch: Some("main".into()),
            revision: None,
            elapsed_ms: 5,
            budget_ms: None,
            source: "test".into(),
        }
    }

    fn count(db: &Database, sql: &str) -> i64 {
        db.conn().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn events(db: &Database, ty: &str) -> Vec<serde_json::Value> {
        let conn = db.conn().unwrap();
        let mut stmt = conn
            .prepare("SELECT payload FROM event_log WHERE type = ?1 ORDER BY seq")
            .unwrap();
        stmt.query_map(params![ty], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|p| serde_json::from_str(&p.unwrap()).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn a_take_writes_snapshot_rows_op_and_event_together() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db.clone());
        // Nothing to record and no baseline: no op at all.
        assert_eq!(
            store
                .record_take(take(1, vec![], SnapshotTrigger::Startup))
                .await
                .unwrap(),
            None
        );
        let mut first = take(
            1,
            vec![("a.txt", "a1"), ("b.txt", "b1")],
            SnapshotTrigger::Startup,
        );
        first.revision = Some(Revision::git("c0ffee"));
        let one = store.record_take(first).await.unwrap().unwrap();
        assert!(!one.unchanged);
        assert_eq!((one.parent_snapshot_id, one.file_count), (None, 2));
        let listed = store
            .list_snapshots_for_stream(StreamId::new(1), 5)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].branch.as_deref(), Some("main"));
        assert_eq!(listed[0].revision, Some(Revision::git("c0ffee")));
        assert_eq!(
            listed[0].tree_hash.as_deref(),
            Some(
                crate::snapshot_tree::manifest_hash(&store.tree_at(one.snapshot_id).await.unwrap())
                    .as_str()
            )
        );

        db.conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (7, 1, 't', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO agent_turn (id, thread_id, prompt, started_at)
                   VALUES (3, 7, 'p', '2026-01-01');",
            )
            .unwrap();
        let mut second = take(1, vec![("a.txt", "a2")], SnapshotTrigger::TurnEnd);
        second.thread_id = Some(ThreadId::new(7));
        second.turn_id = Some(3);
        second.budget_ms = Some(2);
        let two = store.record_take(second).await.unwrap().unwrap();
        assert_eq!(two.parent_snapshot_id, Some(one.snapshot_id));
        assert!(two.over_budget, "5 ms against a 2 ms budget");

        let ops = store.list_ops(StreamId::new(1), 10).await.unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].trigger, SnapshotTrigger::TurnEnd);
        assert_eq!(ops[0].parent_snapshot_id, Some(one.snapshot_id));
        assert_eq!(
            (ops[0].turn_id, ops[0].thread_id),
            (Some(3), Some(ThreadId::new(7)))
        );
        assert!(ops[0].over_budget);
        assert_eq!(ops[0].budget_ms, Some(2));
        let taken = events(&db, "snapshot.taken");
        assert_eq!(taken.len(), 2);
        assert_eq!(
            taken[1]["snapshot"],
            format!("snapshot:{}", two.snapshot_id)
        );
        assert_eq!(taken[1]["parent"], format!("snapshot:{}", one.snapshot_id));
        assert_eq!(taken[1]["trigger"], "turn_end");
        assert_eq!(taken[1]["over_budget"], true);
        let turn_anchor: Option<i64> = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT turn_id FROM event_log WHERE type = 'snapshot.taken' ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(turn_anchor, Some(3));
        assert_eq!(
            store
                .op_for_snapshot(two.snapshot_id)
                .await
                .unwrap()
                .unwrap()
                .seq,
            ops[0].seq
        );
    }

    /// P2.11 (tsk435): a listed snapshot carries what its creating op
    /// recorded — its parent (the "previous" a diff starts from), trigger
    /// and whether it ran over budget; the op log lists newest first.
    #[tokio::test]
    async fn a_listed_snapshot_carries_its_creating_ops_parent_and_trigger() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db);
        let first = store
            .record_take(take(1, vec![("a.txt", "h1")], SnapshotTrigger::Startup))
            .await
            .unwrap()
            .unwrap();
        let mut slow = take(1, vec![("a.txt", "h2")], SnapshotTrigger::TurnEnd);
        slow.budget_ms = Some(1);
        slow.elapsed_ms = 50;
        let second = store.record_take(slow).await.unwrap().unwrap();
        // A later empty take lands on the second snapshot; its creating op
        // is still the turn-end one.
        store
            .record_take(take(1, vec![], SnapshotTrigger::Quiet))
            .await
            .unwrap();

        let rows = store
            .list_snapshots_for_stream(StreamId::new(1), 10)
            .await
            .unwrap();
        let row = |id: i64| rows.iter().find(|r| r.id == id).unwrap().clone();
        let a = row(first.snapshot_id);
        assert_eq!(
            (a.parent_snapshot_id, a.trigger, a.over_budget),
            (None, Some(SnapshotTrigger::Startup), false)
        );
        let b = row(second.snapshot_id);
        assert_eq!(
            (b.parent_snapshot_id, b.trigger, b.over_budget),
            (
                Some(first.snapshot_id),
                Some(SnapshotTrigger::TurnEnd),
                true
            )
        );

        let ops = store.list_ops(StreamId::new(1), 2).await.unwrap();
        let triggers: Vec<SnapshotTrigger> = ops.iter().map(|o| o.trigger).collect();
        assert_eq!(
            triggers,
            vec![SnapshotTrigger::Quiet, SnapshotTrigger::TurnEnd]
        );
    }

    #[tokio::test]
    async fn an_empty_or_identical_take_records_an_op_on_the_parent() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db.clone());
        let base = store
            .record_take(take(1, vec![("a.txt", "a1")], SnapshotTrigger::Startup))
            .await
            .unwrap()
            .unwrap();
        // Nothing dirty.
        let empty = store
            .record_take(take(1, vec![], SnapshotTrigger::TurnEnd))
            .await
            .unwrap()
            .unwrap();
        assert!(empty.unchanged);
        assert_eq!(empty.snapshot_id, base.snapshot_id);
        assert_eq!(empty.parent_snapshot_id, Some(base.snapshot_id));
        // Rows that net out to the same tree (a change and its revert).
        let same = store
            .record_take(take(1, vec![("a.txt", "a1")], SnapshotTrigger::Quiet))
            .await
            .unwrap()
            .unwrap();
        assert!(same.unchanged);
        assert_eq!(same.snapshot_id, base.snapshot_id);
        assert_eq!(count(&db, "SELECT count(*) FROM snapshot"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM file_snapshot"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM snapshot_op"), 3);
        let taken = events(&db, "snapshot.taken");
        assert_eq!(taken.len(), 3);
        assert_eq!(taken[1]["unchanged"], true);
        assert_eq!(taken[1]["file_count"], 0);
    }

    #[tokio::test]
    async fn a_failed_take_leaves_no_snapshot_rows_op_or_event() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db.clone());
        // The second row names a stream that doesn't exist: its insert
        // fails after the snapshot row and the first file row went in.
        let mut bad = take(
            1,
            vec![("a.txt", "a1"), ("b.txt", "b1")],
            SnapshotTrigger::Manual,
        );
        bad.rows[1].stream_id = StreamId::new(999);
        assert!(store.record_take(bad).await.is_err());
        for table in ["snapshot", "file_snapshot", "snapshot_op", "event_log"] {
            assert_eq!(
                count(&db, &format!("SELECT count(*) FROM {table}")),
                0,
                "{table}"
            );
        }
    }

    #[tokio::test]
    async fn a_head_move_restamps_the_current_snapshot_with_an_op_and_event() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db.clone());
        assert_eq!(
            store
                .record_head_moved(StreamId::new(1), 1, Revision::git("aaaaaaa"), "test".into())
                .await
                .unwrap(),
            None,
            "no snapshot yet"
        );
        let mut first = take(1, vec![("a.txt", "a1")], SnapshotTrigger::Startup);
        first.revision = Some(Revision::git("aaaaaaa"));
        let base = store.record_take(first).await.unwrap().unwrap();
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO page_ref (source_kind, source_id, target_kind, target_id, ref_type,
                   local_snapshot_id, vcs_rev_exact)
                 VALUES ('wiki', 'w', 'file', 'a.txt', 'mention', ?1, 0)",
                params![base.snapshot_id],
            )
            .unwrap();
        // Same commit: nothing to record.
        assert_eq!(
            store
                .record_head_moved(
                    StreamId::new(1),
                    base.snapshot_id,
                    Revision::git("aaaaaaa"),
                    "test".into()
                )
                .await
                .unwrap(),
            None
        );
        let moved = store
            .record_head_moved(
                StreamId::new(1),
                base.snapshot_id,
                Revision::git("bbbbbbb"),
                "test".into(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved.snapshot_id, base.snapshot_id);
        assert_eq!(
            store.get_snapshot_revision(base.snapshot_id).await.unwrap(),
            Some(Revision::git("bbbbbbb"))
        );
        let exact: (String, i64) = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT closest_vcs_rev, vcs_rev_exact FROM page_ref",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(exact, ("bbbbbbb".to_string(), 1));
        let ops = store.list_ops(StreamId::new(1), 5).await.unwrap();
        assert_eq!(ops[0].trigger, SnapshotTrigger::HeadMoved);
        // A take landed after the caller saw the clean tree: the stamp is
        // refused rather than claiming the new (dirty) snapshot is HEAD.
        let later = store
            .record_take(take(1, vec![("a.txt", "dirty")], SnapshotTrigger::TurnEnd))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .record_head_moved(
                    StreamId::new(1),
                    base.snapshot_id,
                    Revision::git("ccccccc"),
                    "test".into()
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .get_snapshot_revision(later.snapshot_id)
                .await
                .unwrap(),
            None
        );
        let moved_events = events(&db, "vcs.head.moved");
        assert_eq!(moved_events.len(), 1);
        assert_eq!(moved_events[0]["from"], "commit:aaaaaaa");
        assert_eq!(moved_events[0]["to"], "commit:bbbbbbb");
    }

    #[tokio::test]
    async fn a_revision_stamp_cascades_to_file_refs() {
        // When a revision is stamped on a snapshot, every
        // `effort_file` and `page_ref` row pointing at that
        // snapshot must pick up the sha and flip
        // `vcs_rev_exact` to 1. We bypass the domain stores
        // (FK setup noise) and seed the rows directly.
        let db = Database::in_memory();
        let snap_store = SqliteSnapshotStore::new(db.clone());
        let db_for_snap = db.clone();
        let snap_id: i64 = tokio::task::spawn_blocking(move || {
            db_for_snap.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO streams
                       (id, kind, title, branch, branch_ref, branch_source,
                        worktree_path, created_at, updated_at)
                     VALUES (1, 'primary', 's', 'main', 'refs/heads/main', 'origin',
                             '/tmp', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO snapshot (stream_id, created_at)
                     VALUES (1, '2026-01-01T00:00:00Z')",
                    [],
                )?;
                Ok(conn.last_insert_rowid())
            })
        })
        .await
        .unwrap()
        .unwrap();
        // Seed one effort_file row pointing at this snapshot
        // (skip the FK chain — fk on `effort` is enforced but
        // we can disable it for the test by NOT joining via the
        // store and writing through the raw connection.)
        let db_for_seed = db.clone();
        tokio::task::spawn_blocking(move || {
            db_for_seed.with_conn(|conn| {
                conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
                conn.execute(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                     VALUES ('work_item:oxplow:tsk1', 1, '2026-01-01T00:00:00Z')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO effort_file
                       (effort_id, path, change_kind,
                        local_snapshot_id, closest_vcs_rev, vcs_rev_exact)
                     VALUES (?1, ?2, 'updated', ?3, ?4, 0)",
                    params![1, "src/a.rs", snap_id, "aaaa"],
                )?;
                conn.execute(
                    "INSERT INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type,
                        source_extra, local_snapshot_id, closest_vcs_rev, vcs_rev_exact)
                     VALUES ('wiki', 'intro', 'file', 'src/a.rs', 'wiki_file_ref',
                             NULL, ?1, 'aaaa', 0)",
                    params![snap_id],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();

        snap_store
            .set_snapshot_revision(snap_id, Revision::git("bbbb"))
            .await
            .unwrap();

        let db_for_check = db.clone();
        let (file_sha, file_exact, edge_sha, edge_exact): (
            Option<String>,
            i64,
            Option<String>,
            i64,
        ) = tokio::task::spawn_blocking(move || {
            db_for_check.with_conn(|conn| {
                let mut row = conn.query_row(
                    "SELECT closest_vcs_rev, vcs_rev_exact FROM effort_file
                         WHERE effort_id = 1 AND path = 'src/a.rs'",
                    [],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
                )?;
                let er = conn.query_row(
                    "SELECT closest_vcs_rev, vcs_rev_exact FROM page_ref
                         WHERE source_kind = 'wiki' AND source_id = 'intro'
                           AND target_kind = 'file' AND target_id = 'src/a.rs'",
                    [],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
                )?;
                row = (row.0, row.1);
                Ok((row.0, row.1, er.0, er.1))
            })
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(file_sha.as_deref(), Some("bbbb"));
        assert_eq!(file_exact, 1);
        assert_eq!(edge_sha.as_deref(), Some("bbbb"));
        assert_eq!(edge_exact, 1);
    }

    #[tokio::test]
    async fn set_snapshot_git_branch_is_listed() {
        // The branch a snapshot was captured on round-trips through
        // `set_snapshot_git_branch` → `list_snapshots_for_stream`, so the
        // diff picker can filter to a single branch. Unset snapshots read
        // back as `None`.
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db);
        let stream = StreamId::new(1);

        let on_main = store.create_snapshot(stream).await.unwrap();
        let on_feature = store.create_snapshot(stream).await.unwrap();
        let unstamped = store.create_snapshot(stream).await.unwrap();
        store
            .set_snapshot_branch(on_main, "main".into())
            .await
            .unwrap();
        store
            .set_snapshot_branch(on_feature, "feature-x".into())
            .await
            .unwrap();

        let rows = store.list_snapshots_for_stream(stream, 50).await.unwrap();
        let branch_of = |id: i64| {
            rows.iter()
                .find(|s| s.id == id)
                .and_then(|s| s.branch.clone())
        };
        assert_eq!(branch_of(on_main).as_deref(), Some("main"));
        assert_eq!(branch_of(on_feature).as_deref(), Some("feature-x"));
        assert_eq!(branch_of(unstamped), None);
    }

    #[tokio::test]
    async fn page_visit_record_then_recent() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "abc", None, Some(1234), None)
            .await
            .unwrap();
        store
            .record("task", "wi-1", None, None, None)
            .await
            .unwrap();
        let recent = store.list_recent(10, None).await.unwrap();
        assert_eq!(recent.len(), 2);
        // newest first
        assert_eq!(recent[0].page_kind, "task");
    }

    #[tokio::test]
    async fn page_visit_top_groups_correctly() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "b", None, None, None).await.unwrap();
        let top = store.list_top(10, None).await.unwrap();
        assert_eq!(top[0].1, "a");
        assert_eq!(top[0].2, 2);
    }

    #[tokio::test]
    async fn page_visit_forget_clears_only_target() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "b", None, None, None).await.unwrap();
        store.forget_page("wiki", "a").await.unwrap();
        let recent = store.list_recent(10, None).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].page_id, "b");
    }

    #[tokio::test]
    async fn page_visit_recent_filters_by_thread() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "a", None, None, Some("thr1"))
            .await
            .unwrap();
        store
            .record("wiki", "b", None, None, Some("thr2"))
            .await
            .unwrap();
        store.record("wiki", "c", None, None, None).await.unwrap();
        let in_thread = store.list_recent(10, Some("thr1")).await.unwrap();
        assert_eq!(in_thread.len(), 1);
        assert_eq!(in_thread[0].page_id, "a");
        let global = store.list_recent(10, None).await.unwrap();
        assert_eq!(global.len(), 3);
    }

    #[tokio::test]
    async fn page_visit_top_filters_by_thread() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "a", None, None, Some("thr1"))
            .await
            .unwrap();
        store
            .record("wiki", "a", None, None, Some("thr1"))
            .await
            .unwrap();
        store
            .record("wiki", "a", None, None, Some("thr2"))
            .await
            .unwrap();
        store
            .record("wiki", "b", None, None, Some("thr1"))
            .await
            .unwrap();
        let in_thread = store.list_top(10, Some("thr1")).await.unwrap();
        // a appears twice in b-1, b once
        assert_eq!(in_thread[0].1, "a");
        assert_eq!(in_thread[0].2, 2);
        assert_eq!(in_thread[1].1, "b");
        assert_eq!(in_thread[1].2, 1);
    }

    #[tokio::test]
    async fn usage_event_round_trip() {
        let store = SqliteUsageStore::new(Database::in_memory());
        store
            .record("agent_turn_started", serde_json::json!({"thread": "b-1"}))
            .await
            .unwrap();
        let recent = store.list_recent(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].kind, "agent_turn_started");
    }

    #[tokio::test]
    async fn usage_rollup_groups_by_key_and_orders_by_recency() {
        let store = SqliteUsageStore::new(Database::in_memory());
        // Two hits on a.ts in stream s-1, one on b.ts (s-1), one on
        // c.ts in a different stream s-2 (must be filtered out).
        for path in ["a.ts", "a.ts", "b.ts"] {
            store
                .record(
                    "editor-file",
                    serde_json::json!({"path": path, "streamId": "s-1"}),
                )
                .await
                .unwrap();
        }
        store
            .record(
                "editor-file",
                serde_json::json!({"path": "c.ts", "streamId": "s-2"}),
            )
            .await
            .unwrap();
        // A different kind in the same stream — must not appear.
        store
            .record("wiki", serde_json::json!({"slug": "z", "streamId": "s-1"}))
            .await
            .unwrap();

        let rollup = store
            .list_recent_rollup("editor-file", Some("s-1"), 10)
            .await
            .unwrap();
        assert_eq!(rollup.len(), 2);
        // b.ts was inserted last → most recent → first.
        assert_eq!(rollup[0].key, "b.ts");
        assert_eq!(rollup[0].count, 1);
        assert_eq!(rollup[1].key, "a.ts");
        assert_eq!(rollup[1].count, 2);
        for r in &rollup {
            assert_eq!(r.kind, "editor-file");
        }

        // Without stream filter: c.ts also shows up.
        let global = store
            .list_recent_rollup("editor-file", None, 10)
            .await
            .unwrap();
        assert_eq!(global.len(), 3);
    }

    #[tokio::test]
    async fn usage_rollup_drops_rows_with_no_extractable_key() {
        let store = SqliteUsageStore::new(Database::in_memory());
        store
            .record("editor-file", serde_json::json!({"streamId": "s-1"}))
            .await
            .unwrap();
        store
            .record(
                "editor-file",
                serde_json::json!({"path": "real.ts", "streamId": "s-1"}),
            )
            .await
            .unwrap();
        let rollup = store
            .list_recent_rollup("editor-file", Some("s-1"), 10)
            .await
            .unwrap();
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].key, "real.ts");
    }

    #[tokio::test]
    async fn code_quality_scan_lifecycle() {
        let store = SqliteCodeQualityStore::new(Database::in_memory());
        let id = store
            .create_scan("metrics", "workspace", "working", "all")
            .await
            .unwrap();
        store
            .finish_scan_with_findings(
                id,
                vec![CodeQualityFinding {
                    id: 0,
                    scan_id: id,
                    path: "src/main.rs".into(),
                    start_line: 10,
                    end_line: 50,
                    kind: "complexity".into(),
                    metric_value: 14.0,
                    extra_json: None,
                }],
            )
            .await
            .unwrap();
        let scans = store.list_scans(10).await.unwrap();
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].status, CodeQualityScanStatus::Done);
        let findings = store.list_findings(id).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].metric_value, 14.0);
    }

    /// A scan's findings land with it in one write, and a newer finished
    /// scan of the same tool and scope replaces the older ones — their
    /// findings and file edges with them — while other scopes stay.
    #[tokio::test]
    async fn a_finished_scan_replaces_its_scopes_older_scans() {
        let db = Database::in_memory();
        let store = SqliteCodeQualityStore::new(db.clone());
        let finding = |path: &str| CodeQualityFinding {
            id: 0,
            scan_id: 0,
            path: path.into(),
            start_line: 1,
            end_line: 12,
            kind: "duplicate-block".into(),
            metric_value: 12.0,
            extra_json: None,
        };
        let scan = |scope: &'static str| {
            let store = &store;
            async move {
                store
                    .create_scan("duplication", scope, "working", "x")
                    .await
                    .unwrap()
            }
        };
        let old = scan("change 1").await;
        store
            .finish_scan_with_findings(old, vec![finding("a.rs"), finding("b.rs")])
            .await
            .unwrap();
        let other = scan("change 2").await;
        store
            .finish_scan_with_findings(other, vec![finding("c.rs")])
            .await
            .unwrap();
        let new = scan("change 1").await;
        store
            .finish_scan_with_findings(new, vec![finding("a.rs")])
            .await
            .unwrap();

        let mut ids: Vec<i64> = store
            .list_scans(10)
            .await
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec![other, new]);
        assert_eq!(
            store.list_scans(10).await.unwrap()[0].status,
            CodeQualityScanStatus::Done
        );
        assert_eq!(store.list_findings(new).await.unwrap().len(), 1);
        assert!(store.list_findings(old).await.unwrap().is_empty());
        let edges: i64 = db
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM page_ref WHERE source_kind = 'finding'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            edges, 2,
            "one edge each for the new scan's and change 2's findings"
        );
    }

    #[tokio::test]
    async fn snapshot_capture_then_list() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db.clone());
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: StreamId::new(1),
                path: "src/foo.rs".into(),
                blob_hash: Some("abc".into()),
                size_bytes: 42,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: None,
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        let list = store.list_for_path("src/foo.rs").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].size_bytes, 42);
        // The same row, published as `v_snapshot_file` (tsk944).
        let published = db
            .read(|tx| {
                let rows = || -> rusqlite::Result<Vec<(String, String, i64)>> {
                    let mut st =
                        tx.prepare("SELECT path, storage, size_bytes FROM v_snapshot_file")?;
                    let rows = st
                        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(rows)
                };
                rows().map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(
            published,
            vec![("src/foo.rs".to_string(), "oxplow".to_string(), 42)]
        );
    }

    #[tokio::test]
    async fn capture_round_trips_storage_class() {
        let db = Database::in_memory();
        // file_snapshot.stream_id FK → seed a stream first.
        seed_snapshots(&db, &[(1, "seed.txt", Some("h0"), false)]).await;
        let store = SqliteSnapshotStore::new(db);
        // A git-backed row: blob_hash holds the git OID, storage = Git.
        let id = store
            .capture(FileSnapshot {
                id: 0,
                stream_id: StreamId::new(1),
                path: "src/clean.rs".into(),
                blob_hash: Some("deadbeefcafe".into()),
                size_bytes: 10,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Git,
                snapshot_id: None,
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        let got = store.get(id).await.unwrap().unwrap();
        assert_eq!(got.storage, SnapshotStorage::Git);
        assert_eq!(got.blob_hash.as_deref(), Some("deadbeefcafe"));
        assert!(got.storage.has_bytes());

        // A deletion tombstone: NULL hash, storage = Deleted.
        let del = store
            .capture(FileSnapshot {
                id: 0,
                stream_id: StreamId::new(1),
                path: "src/clean.rs".into(),
                blob_hash: None,
                size_bytes: 0,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Deleted,
                snapshot_id: None,
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        let got = store.get(del).await.unwrap().unwrap();
        assert_eq!(got.storage, SnapshotStorage::Deleted);
        assert!(!got.storage.has_bytes());
    }

    #[tokio::test]
    async fn stats_for_snapshot_classifies_created_modified_deleted() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db);
        let stream = StreamId::new(1);

        // Parent 1: baseline of three paths.
        let p1 = store.create_snapshot(stream).await.unwrap();
        for path in ["a.txt", "b.txt", "c.txt"] {
            store
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream,
                    path: path.into(),
                    blob_hash: Some(format!("h-{path}-v1")),
                    size_bytes: 10,
                    captured_at: Timestamp::now(),
                    storage: SnapshotStorage::Oxplow,
                    snapshot_id: Some(p1),
                    mtime_ms: None,
                    content_hash: None,
                })
                .await
                .unwrap();
        }

        // Parent 2: a modified, c deleted (a tombstone row), d created.
        let p2 = store.create_snapshot(stream).await.unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "a.txt".into(),
                blob_hash: Some("h-a.txt-v2".into()),
                size_bytes: 20,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "c.txt".into(),
                blob_hash: None,
                size_bytes: 0,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Deleted,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "d.txt".into(),
                blob_hash: Some("h-d.txt-v1".into()),
                size_bytes: 5,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();

        let summary = store.stats_for_snapshot(p2).await.unwrap();
        assert_eq!(summary.created, 1, "d.txt is new");
        assert_eq!(summary.modified, 1, "a.txt had a prior hash");
        assert_eq!(summary.deleted, 1, "c.txt has no blob");
        assert_eq!(summary.total, 3);

        // p1 itself: three created rows, nothing else.
        let p1_summary = store.stats_for_snapshot(p1).await.unwrap();
        assert_eq!(p1_summary.created, 3);
        assert_eq!(p1_summary.modified, 0);
        assert_eq!(p1_summary.deleted, 0);
        assert_eq!(p1_summary.total, 3);
    }

    #[tokio::test]
    async fn list_changes_for_snapshot_carries_status_and_prior_id() {
        let db = Database::in_memory();
        seed_stream(&db, 1);
        let store = SqliteSnapshotStore::new(db);
        let stream = StreamId::new(1);

        // p1: a and b baselined.
        let p1 = store.create_snapshot(stream).await.unwrap();
        let a1 = store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "a.txt".into(),
                blob_hash: Some("h-a-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "b.txt".into(),
                blob_hash: Some("h-b-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p1),
                mtime_ms: None,
                content_hash: None,
            })
            .await
            .unwrap();

        // p2: a modified, c added, b deleted.
        let p2 = store.create_snapshot(stream).await.unwrap();
        for snap in [
            FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "a.txt".into(),
                blob_hash: Some("h-a-v2".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            },
            FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "c.txt".into(),
                blob_hash: Some("h-c-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Oxplow,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            },
            FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "b.txt".into(),
                blob_hash: None,
                size_bytes: 0,
                captured_at: Timestamp::now(),
                storage: SnapshotStorage::Deleted,
                snapshot_id: Some(p2),
                mtime_ms: None,
                content_hash: None,
            },
        ] {
            store.capture(snap).await.unwrap();
        }

        let entries = store.list_changes_for_snapshot(p2).await.unwrap();
        let by_path: std::collections::HashMap<_, _> =
            entries.iter().map(|e| (e.path.clone(), e)).collect();
        assert_eq!(by_path["a.txt"].status, "modified");
        assert_eq!(by_path["a.txt"].prior_file_id, Some(a1));
        assert_eq!(by_path["c.txt"].status, "added");
        assert_eq!(by_path["c.txt"].prior_file_id, None);
        assert_eq!(by_path["b.txt"].status, "deleted");
        assert!(by_path["b.txt"].prior_file_id.is_some());
    }

    fn seed_stream(db: &Database, id: i64) {
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES (?1, 'primary', 't', 'main', 'refs/heads/main', 'main', '/r', '2026-01-01', '2026-01-01')",
            params![id],
        ).unwrap();
    }
}
