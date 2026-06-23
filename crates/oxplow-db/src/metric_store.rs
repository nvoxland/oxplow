//! Unified metric substrate store (epic tsk213, phase P0).
//!
//! Backs the typed metric model from `V38__metrics.sql`:
//!   * [`MetricDefinition`] — the measure catalog (one row per known metric).
//!   * [`MetricRun`] — an optional compute event that produces samples/findings.
//!   * [`MetricSample`] — the durable scalar fact (the BI grain). TIME-PRIMARY:
//!     anchored by `captured_at` + `closest_git_version`, with NO effort FK, so
//!     efforts/commits are time-range overlays and samples outlive them.
//!   * [`MetricFinding`] — located detail for the `findings` kind.
//!
//! Modeled on `observation_store.rs` / `analytics_stores.rs` (sync work inside
//! `Database::call`). Raw integer ids are used at this layer; the service/IPC
//! layer maps to/from the prefixed domain ids (`str1`, `eff1`, …).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{canonical_ts, map_sql_err, Database};

fn ts_to_string(ts: Timestamp) -> String {
    let raw = serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string();
    // Fixed-width canonical form so SQLite's lexicographic ORDER BY / range
    // comparisons on `captured_at` match chronological order (tsk243).
    canonical_ts(&raw)
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn ts_conv_err(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

// ---------------------------------------------------------------------------
// Definition (the measure catalog)
// ---------------------------------------------------------------------------

/// One row in the measure catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricDefinition {
    pub id: i64,
    pub key: String,
    /// `gauge` | `findings` | `test` | `coverage` | `event`.
    pub kind: String,
    pub title: String,
    pub unit: Option<String>,
    /// `higher-better` | `lower-better` | `neutral`.
    pub direction: String,
    /// `last` | `sum` | `avg` | `min` | `max`.
    pub default_agg: String,
    pub grain: Option<String>,
    pub basis: String,
    pub producer: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    /// `built-in` | `global` | `project`.
    pub scope: String,
    /// JSON array of declared conformed-dimension keys.
    pub dimensions_json: Option<String>,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Write-side input for [`SqliteMetricStore::upsert_definition`]. Build with
/// [`NewMetricDefinition::new`] then override fields.
#[derive(Debug, Clone)]
pub struct NewMetricDefinition {
    pub key: String,
    pub kind: String,
    pub title: String,
    pub unit: Option<String>,
    pub direction: String,
    pub default_agg: String,
    pub grain: Option<String>,
    pub basis: String,
    pub producer: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub scope: String,
    pub dimensions_json: Option<String>,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
}

impl NewMetricDefinition {
    /// A definition with sensible defaults: `neutral` direction, `last`
    /// aggregation, `absolute` basis, `built-in` scope.
    pub fn new(key: impl Into<String>, kind: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind: kind.into(),
            title: title.into(),
            unit: None,
            direction: "neutral".into(),
            default_agg: "last".into(),
            grain: None,
            basis: "absolute".into(),
            producer: None,
            description: None,
            category: None,
            language: None,
            scope: "built-in".into(),
            dimensions_json: None,
            target: None,
            warn_at: None,
            fail_at: None,
        }
    }
}

const DEF_COLS: &str = "id, key, kind, title, unit, direction, default_agg, grain, basis, \
     producer, description, category, language, scope, dimensions_json, target, warn_at, \
     fail_at, created_at, updated_at";

fn row_to_definition(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricDefinition> {
    let created_at: String = row.get(18)?;
    let updated_at: String = row.get(19)?;
    Ok(MetricDefinition {
        id: row.get(0)?,
        key: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        unit: row.get(4)?,
        direction: row.get(5)?,
        default_agg: row.get(6)?,
        grain: row.get(7)?,
        basis: row.get(8)?,
        producer: row.get(9)?,
        description: row.get(10)?,
        category: row.get(11)?,
        language: row.get(12)?,
        scope: row.get(13)?,
        dimensions_json: row.get(14)?,
        target: row.get(15)?,
        warn_at: row.get(16)?,
        fail_at: row.get(17)?,
        created_at: string_to_ts(&created_at).map_err(ts_conv_err)?,
        updated_at: string_to_ts(&updated_at).map_err(ts_conv_err)?,
    })
}

// ---------------------------------------------------------------------------
// Run (compute event)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricRun {
    pub id: i64,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub producer: String,
    pub status: String,
    pub error: Option<String>,
    pub scope: Option<String>,
    pub trigger: Option<String>,
    pub basis_ref: Option<String>,
    pub provenance: String,
    pub source: String,
    pub snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    /// Branch the run was captured on, when applicable.
    pub branch: Option<String>,
    pub git_version_exact: bool,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct NewMetricRun {
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub producer: String,
    pub status: String,
    pub error: Option<String>,
    pub scope: Option<String>,
    pub trigger: Option<String>,
    pub basis_ref: Option<String>,
    pub provenance: String,
    pub source: String,
    pub snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub branch: Option<String>,
    pub git_version_exact: bool,
    /// Defaults to now when `None`.
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
}

impl NewMetricRun {
    /// A completed (`status = done`, `provenance = observed`) run for `producer`.
    pub fn done(stream_id: i64, producer: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            stream_id,
            thread_id: None,
            producer: producer.into(),
            status: "done".into(),
            error: None,
            scope: None,
            trigger: None,
            basis_ref: None,
            provenance: "observed".into(),
            source: source.into(),
            snapshot_id: None,
            closest_git_version: None,
            branch: None,
            git_version_exact: false,
            started_at: None,
            ended_at: None,
        }
    }
}

const RUN_COLS: &str = "id, stream_id, thread_id, producer, status, error, scope, trigger, \
     basis_ref, provenance, source, snapshot_id, closest_git_version, git_version_exact, \
     started_at, ended_at, branch";

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricRun> {
    let started_at: String = row.get(14)?;
    let ended_at: Option<String> = row.get(15)?;
    Ok(MetricRun {
        id: row.get(0)?,
        stream_id: row.get(1)?,
        thread_id: row.get(2)?,
        producer: row.get(3)?,
        status: row.get(4)?,
        error: row.get(5)?,
        scope: row.get(6)?,
        trigger: row.get(7)?,
        basis_ref: row.get(8)?,
        provenance: row.get(9)?,
        source: row.get(10)?,
        snapshot_id: row.get(11)?,
        closest_git_version: row.get(12)?,
        git_version_exact: row.get::<_, i64>(13)? != 0,
        branch: row.get(16)?,
        started_at: string_to_ts(&started_at).map_err(ts_conv_err)?,
        ended_at: match ended_at {
            Some(s) => Some(string_to_ts(&s).map_err(ts_conv_err)?),
            None => None,
        },
    })
}

// ---------------------------------------------------------------------------
// Sample (the durable scalar fact)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricSample {
    pub id: i64,
    pub run_id: Option<i64>,
    pub metric_id: i64,
    pub value: f64,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub captured_at: Timestamp,
    pub snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    /// Branch the fact was captured on, when applicable.
    pub branch: Option<String>,
    pub git_version_exact: bool,
    pub basis_ref: Option<String>,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub dims_json: Option<String>,
    pub provenance: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct NewMetricSample {
    pub run_id: Option<i64>,
    pub metric_id: i64,
    pub value: f64,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    /// Defaults to now when `None`.
    pub captured_at: Option<Timestamp>,
    pub snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub branch: Option<String>,
    pub git_version_exact: bool,
    pub basis_ref: Option<String>,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub dims_json: Option<String>,
    pub provenance: String,
    pub source: String,
}

impl NewMetricSample {
    /// A minimal `observed` scalar sample of `metric_id` in `stream_id`.
    pub fn observed(metric_id: i64, stream_id: i64, value: f64, source: impl Into<String>) -> Self {
        Self {
            run_id: None,
            metric_id,
            value,
            numerator: None,
            denominator: None,
            captured_at: None,
            snapshot_id: None,
            closest_git_version: None,
            branch: None,
            git_version_exact: false,
            basis_ref: None,
            stream_id,
            thread_id: None,
            subject_kind: None,
            subject_ref: None,
            path: None,
            line: None,
            dims_json: None,
            provenance: "observed".into(),
            source: source.into(),
        }
    }

    /// A minimal `asserted` scalar sample — a value the agent/CI reported
    /// rather than one oxplow computed itself (lower trust). `source` describes
    /// who asserted it (e.g. `"agent-reported"`).
    pub fn asserted(metric_id: i64, stream_id: i64, value: f64, source: impl Into<String>) -> Self {
        Self {
            provenance: "asserted".into(),
            ..Self::observed(metric_id, stream_id, value, source)
        }
    }
}

/// One metric's roll-up over a single effort — the wire shape the task/effort
/// page reads (built by `CollectionService::effort_metric_deltas`). NOT a stored
/// row: derived per request from the substrate using the right attribution key
/// per metric family (file-attributed for gauges, thread-scoped for operational,
/// effort-diff for coverage/tests). See metrics.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortMetricDelta {
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    /// `higher-better` | `lower-better` | `neutral`.
    pub direction: String,
    /// The definition `kind` (`gauge` | `coverage` | `test` | `event` | …).
    pub kind: String,
    pub category: Option<String>,
    pub language: Option<String>,
    /// How this delta was computed: `files` (Σ over the effort's claimed files),
    /// `sum` (Σ in-window flow, e.g. tokens), or `level` (before→after).
    pub agg: String,
    /// The value as the effort began (`None` for a `sum`/flow metric).
    pub baseline: Option<f64>,
    /// The value as of the effort's end (or latest, if open).
    pub current: f64,
    /// `current − baseline` for a level/file metric; the flow total for `sum`.
    pub delta: Option<f64>,
    /// Whether the value moved across the effort (false ⇒ show the value only).
    pub changed: bool,
    /// For `files`: how many of the effort's claimed files carry this metric.
    pub attributed_files: Option<i64>,
    /// Samples considered (in-window, or per-file for `files`).
    pub sample_count: i64,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    /// `warn` | `fail` when `current` (the repo-total headline for gauges) sits
    /// in that zone, interpreted via `direction`; else `None`.
    pub crossing: Option<String>,
    /// The latest contributing run, for findings drill-in.
    pub latest_run_id: Option<i64>,
}

const SAMPLE_COLS: &str = "id, run_id, metric_id, value, numerator, denominator, captured_at, \
     snapshot_id, closest_git_version, git_version_exact, basis_ref, stream_id, thread_id, \
     subject_kind, subject_ref, path, line, dims_json, provenance, source, branch";

/// Insert one `metric_run` row (shared by `record_run` and the transactional
/// `record_run_with_data`). `started_at` defaults to now.
fn insert_run(conn: &rusqlite::Connection, run: NewMetricRun) -> rusqlite::Result<i64> {
    let started = run
        .started_at
        .map(ts_to_string)
        .unwrap_or_else(|| ts_to_string(Timestamp::now()));
    let ended = run.ended_at.map(ts_to_string);
    conn.execute(
        "INSERT INTO metric_run
           (stream_id, thread_id, producer, status, error, scope, trigger, basis_ref,
            provenance, source, snapshot_id, closest_git_version, git_version_exact,
            started_at, ended_at, branch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            run.stream_id,
            run.thread_id,
            run.producer,
            run.status,
            run.error,
            run.scope,
            run.trigger,
            run.basis_ref,
            run.provenance,
            run.source,
            run.snapshot_id,
            run.closest_git_version,
            run.git_version_exact,
            started,
            ended,
            run.branch,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Insert one `metric_sample` row. `captured_at` defaults to now.
fn insert_sample(conn: &rusqlite::Connection, s: NewMetricSample) -> rusqlite::Result<i64> {
    let captured = s
        .captured_at
        .map(ts_to_string)
        .unwrap_or_else(|| ts_to_string(Timestamp::now()));
    conn.execute(
        "INSERT INTO metric_sample
           (run_id, metric_id, value, numerator, denominator, captured_at, snapshot_id,
            closest_git_version, git_version_exact, basis_ref, stream_id, thread_id,
            subject_kind, subject_ref, path, line, dims_json, provenance, source, branch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 ?16, ?17, ?18, ?19, ?20)",
        params![
            s.run_id,
            s.metric_id,
            s.value,
            s.numerator,
            s.denominator,
            captured,
            s.snapshot_id,
            s.closest_git_version,
            s.git_version_exact,
            s.basis_ref,
            s.stream_id,
            s.thread_id,
            s.subject_kind,
            s.subject_ref,
            s.path,
            s.line,
            s.dims_json,
            s.provenance,
            s.source,
            s.branch,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Insert one `metric_finding` row.
fn insert_finding(conn: &rusqlite::Connection, f: NewMetricFinding) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO metric_finding
           (run_id, metric_id, subject_kind, subject_ref, path, start_line, end_line,
            col, kind, severity, rule, message, value, extra_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            f.run_id,
            f.metric_id,
            f.subject_kind,
            f.subject_ref,
            f.path,
            f.start_line,
            f.end_line,
            f.col,
            f.kind,
            f.severity,
            f.rule,
            f.message,
            f.value,
            f.extra_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn row_to_sample(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricSample> {
    let captured_at: String = row.get(6)?;
    Ok(MetricSample {
        id: row.get(0)?,
        run_id: row.get(1)?,
        metric_id: row.get(2)?,
        value: row.get(3)?,
        numerator: row.get(4)?,
        denominator: row.get(5)?,
        captured_at: string_to_ts(&captured_at).map_err(ts_conv_err)?,
        snapshot_id: row.get(7)?,
        closest_git_version: row.get(8)?,
        git_version_exact: row.get::<_, i64>(9)? != 0,
        basis_ref: row.get(10)?,
        stream_id: row.get(11)?,
        thread_id: row.get(12)?,
        subject_kind: row.get(13)?,
        subject_ref: row.get(14)?,
        path: row.get(15)?,
        line: row.get(16)?,
        dims_json: row.get(17)?,
        provenance: row.get(18)?,
        source: row.get(19)?,
        branch: row.get(20)?,
    })
}

// ---------------------------------------------------------------------------
// Finding (located detail)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricFinding {
    pub id: i64,
    pub run_id: i64,
    pub metric_id: Option<i64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub col: Option<i64>,
    pub kind: String,
    pub severity: Option<String>,
    pub rule: Option<String>,
    pub message: Option<String>,
    pub value: Option<f64>,
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewMetricFinding {
    pub run_id: i64,
    pub metric_id: Option<i64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub col: Option<i64>,
    pub kind: String,
    pub severity: Option<String>,
    pub rule: Option<String>,
    pub message: Option<String>,
    pub value: Option<f64>,
    pub extra_json: Option<String>,
}

const FINDING_COLS: &str = "id, run_id, metric_id, subject_kind, subject_ref, path, start_line, \
     end_line, col, kind, severity, rule, message, value, extra_json";

fn row_to_finding(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricFinding> {
    Ok(MetricFinding {
        id: row.get(0)?,
        run_id: row.get(1)?,
        metric_id: row.get(2)?,
        subject_kind: row.get(3)?,
        subject_ref: row.get(4)?,
        path: row.get(5)?,
        start_line: row.get(6)?,
        end_line: row.get(7)?,
        col: row.get(8)?,
        kind: row.get(9)?,
        severity: row.get(10)?,
        rule: row.get(11)?,
        message: row.get(12)?,
        value: row.get(13)?,
        extra_json: row.get(14)?,
    })
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SqliteMetricStore {
    db: Database,
}

impl SqliteMetricStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert or update (by `key`) a metric definition; returns its row id.
    /// `created_at` is preserved across updates.
    pub async fn upsert_definition(&self, def: NewMetricDefinition) -> Result<i64, DomainError> {
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO metric_definition
                       (key, kind, title, unit, direction, default_agg, grain, basis, producer,
                        description, category, language, scope, dimensions_json, target, warn_at,
                        fail_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16, ?17, ?18, ?18)
                     ON CONFLICT(key) DO UPDATE SET
                        kind=excluded.kind, title=excluded.title, unit=excluded.unit,
                        direction=excluded.direction, default_agg=excluded.default_agg,
                        grain=excluded.grain, basis=excluded.basis, producer=excluded.producer,
                        description=excluded.description, category=excluded.category,
                        language=excluded.language, scope=excluded.scope,
                        dimensions_json=excluded.dimensions_json, target=excluded.target,
                        warn_at=excluded.warn_at, fail_at=excluded.fail_at,
                        updated_at=excluded.updated_at",
                    params![
                        def.key,
                        def.kind,
                        def.title,
                        def.unit,
                        def.direction,
                        def.default_agg,
                        def.grain,
                        def.basis,
                        def.producer,
                        def.description,
                        def.category,
                        def.language,
                        def.scope,
                        def.dimensions_json,
                        def.target,
                        def.warn_at,
                        def.fail_at,
                        now,
                    ],
                )?;
                conn.query_row(
                    "SELECT id FROM metric_definition WHERE key = ?1",
                    params![def.key],
                    |r| r.get(0),
                )
            })
            .await
    }

    pub async fn get_definition(&self, key: &str) -> Result<Option<MetricDefinition>, DomainError> {
        let key = key.to_string();
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {DEF_COLS} FROM metric_definition WHERE key = ?1");
                conn.query_row(&sql, params![key], row_to_definition)
                    .optional()
            })
            .await
    }

    pub async fn list_definitions(&self) -> Result<Vec<MetricDefinition>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {DEF_COLS} FROM metric_definition ORDER BY key");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_definition)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Insert a run; returns its row id. `started_at` defaults to now.
    pub async fn record_run(&self, run: NewMetricRun) -> Result<i64, DomainError> {
        self.db.call(move |conn| insert_run(conn, run)).await
    }

    /// Atomically insert a run plus all of its dependent samples and findings in
    /// one transaction. Each sample/finding's `run_id` is forced to the new
    /// run's id, so a producer cannot leave a half-written graph behind on a
    /// crash mid-write (a run with no samples, or samples missing the `*-detail`
    /// finding the effort panel reconstructs from). Returns the run id.
    pub async fn record_run_with_data(
        &self,
        run: NewMetricRun,
        samples: Vec<NewMetricSample>,
        findings: Vec<NewMetricFinding>,
    ) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let run_id = insert_run(&tx, run).map_err(map_sql_err)?;
                for mut s in samples {
                    s.run_id = Some(run_id);
                    insert_sample(&tx, s).map_err(map_sql_err)?;
                }
                for mut f in findings {
                    f.run_id = run_id;
                    insert_finding(&tx, f).map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(run_id)
            })
            .await
    }

    /// Mark a previously-started run finished (`status`, optional `error`,
    /// `ended_at` = now).
    pub async fn finish_run(
        &self,
        run_id: i64,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DomainError> {
        let status = status.to_string();
        let error = error.map(str::to_string);
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "UPDATE metric_run SET status = ?2, error = ?3, ended_at = ?4 WHERE id = ?1",
                    params![run_id, status, error, now],
                )?;
                Ok(())
            })
            .await
    }

    /// Insert a scalar sample; returns its row id. `captured_at` defaults to now.
    pub async fn record_sample(&self, s: NewMetricSample) -> Result<i64, DomainError> {
        self.db.call(move |conn| insert_sample(conn, s)).await
    }

    /// Insert a finding; returns its row id.
    pub async fn record_finding(&self, f: NewMetricFinding) -> Result<i64, DomainError> {
        self.db.call(move |conn| insert_finding(conn, f)).await
    }

    /// Fetch a run by id.
    pub async fn get_run(&self, run_id: i64) -> Result<Option<MetricRun>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {RUN_COLS} FROM metric_run WHERE id = ?1");
                conn.query_row(&sql, params![run_id], row_to_run).optional()
            })
            .await
    }

    /// Runs for a stream, newest-first (optionally filtered to one producer).
    pub async fn list_runs(
        &self,
        stream_id: i64,
        producer: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MetricRun>, DomainError> {
        let producer = producer.map(str::to_string);
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {RUN_COLS} FROM metric_run
                      WHERE stream_id = ?1 AND (?2 IS NULL OR producer = ?2)
                      ORDER BY started_at DESC, id DESC LIMIT ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![stream_id, producer, limit], row_to_run)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Runs of `producer` on `thread_id` whose `started_at` falls in an effort's
    /// time window `[start, end]` (open effort ⇒ `end = None`). The OBSERVE side
    /// for the run attribution kinds (test/coverage/analysis) — `metric_run`
    /// already carries thread + time, so a run is observed against an effort by
    /// thread + window, then attributed by claim+reconcile (tsk262). A thread
    /// belongs to one stream, so `thread_id` alone scopes it. Oldest-first.
    pub async fn runs_in_window(
        &self,
        thread_id: i64,
        producer: &str,
        start: Timestamp,
        end: Option<Timestamp>,
    ) -> Result<Vec<MetricRun>, DomainError> {
        let producer = producer.to_string();
        let start = ts_to_string(start);
        let end = end.map(ts_to_string);
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {RUN_COLS} FROM metric_run
                      WHERE thread_id = ?1 AND producer = ?2
                        AND started_at >= ?3
                        AND (?4 IS NULL OR started_at <= ?4)
                      ORDER BY started_at ASC, id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread_id, producer, start, end], row_to_run)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// All **headline** samples for a metric, newest-first. Per-file attribution
    /// samples (`subject_kind = 'file'`) are excluded — they're the effort-
    /// rollup's grain (`file_samples_for_paths`), not the headline time series
    /// the Metrics page / trend / summary read. See metrics.md (grain split).
    pub async fn list_samples(&self, metric_id: i64) -> Result<Vec<MetricSample>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SAMPLE_COLS} FROM metric_sample WHERE metric_id = ?1
                        AND (subject_kind IS NULL OR subject_kind != 'file')
                      ORDER BY captured_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![metric_id], row_to_sample)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Samples of `metric_id` that fall within the time window of `effort_id`
    /// (the effort-as-overlay model: efforts are NOT a stored sample dimension;
    /// membership is `captured_at` ∈ [started_at, ended_at]). An open effort
    /// (no `ended_at`) includes everything from its start onward.
    pub async fn samples_for_effort(
        &self,
        metric_id: i64,
        effort_id: i64,
    ) -> Result<Vec<MetricSample>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SAMPLE_COLS} FROM metric_sample s
                      WHERE s.metric_id = ?1
                        AND (s.subject_kind IS NULL OR s.subject_kind != 'file')
                        AND s.captured_at >= (SELECT started_at FROM task_effort WHERE id = ?2)
                        AND (
                          (SELECT ended_at FROM task_effort WHERE id = ?2) IS NULL
                          OR s.captured_at <= (SELECT ended_at FROM task_effort WHERE id = ?2)
                        )
                      ORDER BY s.captured_at ASC, s.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![metric_id, effort_id], row_to_sample)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Per-file attribution samples (`subject_kind = 'file'`) for a metric,
    /// scoped to `stream_id` and matched against `paths` on `subject_ref`,
    /// oldest-first. The effort rollup groups these by path and windows them in
    /// Rust to compute each claimed file's before→after contribution — the
    /// attribution grain that disentangles overlapping efforts. Empty when
    /// `paths` is empty. `stream_id` is the hard scope (a file changed on two
    /// worktrees has samples under two streams).
    pub async fn file_samples_for_paths(
        &self,
        metric_id: i64,
        stream_id: i64,
        paths: Vec<String>,
    ) -> Result<Vec<MetricSample>, DomainError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |conn| {
                let placeholders = std::iter::repeat("?")
                    .take(paths.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT {SAMPLE_COLS} FROM metric_sample
                      WHERE metric_id = ? AND stream_id = ? AND subject_kind = 'file'
                        AND subject_ref IN ({placeholders})
                      ORDER BY captured_at ASC, id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&metric_id, &stream_id];
                for p in &paths {
                    binds.push(p);
                }
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_sample)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Findings for a run.
    pub async fn list_findings(&self, run_id: i64) -> Result<Vec<MetricFinding>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FINDING_COLS} FROM metric_finding WHERE run_id = ?1
                      ORDER BY id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![run_id], row_to_finding)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Correctly aggregate a ratio metric by re-combining stored components:
    /// `sum(numerator) / sum(denominator)`. Returns `None` when no sample
    /// carries components or the denominators sum to zero. This is what makes
    /// "coverage % by module" right instead of a naive average of per-file %s.
    pub async fn aggregate_ratio(&self, metric_id: i64) -> Result<Option<f64>, DomainError> {
        self.db
            .call(move |conn| {
                let row: Option<(Option<f64>, Option<f64>)> = conn
                    .query_row(
                        "SELECT SUM(numerator), SUM(denominator) FROM metric_sample
                          WHERE metric_id = ?1 AND numerator IS NOT NULL AND denominator IS NOT NULL",
                        params![metric_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                Ok(match row {
                    Some((Some(num), Some(den))) if den != 0.0 => Some(num / den),
                    _ => None,
                })
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stream(1) + thread(1) + task + effort so run/sample FKs resolve and the
    /// effort-overlay query has a window to read.
    async fn fixture() -> SqliteMetricStore {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-05-26T00:00:00Z";
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                     VALUES (1, 1, 't', 'active', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES (1, 't', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                let task_id = conn.last_insert_rowid();
                // Effort window [start, end] = [10:00, 11:00] for the overlay
                // test. Stored in the same fixed-width canonical form the effort
                // store now writes (tsk243), so sub-second boundary comparisons
                // are exercised faithfully.
                conn.execute(
                    "INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at)
                     VALUES (1, ?1, 1, '2026-05-26T10:00:00.000000Z', '2026-05-26T11:00:00.000000Z')",
                    params![task_id],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        SqliteMetricStore::new(db)
    }

    async fn gauge_def(store: &SqliteMetricStore, key: &str) -> i64 {
        store
            .upsert_definition(NewMetricDefinition::new(key, "gauge", key))
            .await
            .unwrap()
    }

    fn at(ts: &str) -> Timestamp {
        string_to_ts(ts).unwrap()
    }

    #[tokio::test]
    async fn upsert_definition_inserts_then_updates_in_place() {
        let store = fixture().await;
        let mut def =
            NewMetricDefinition::new("oxplow.coverage.diff_pct", "coverage", "Diff coverage");
        def.direction = "higher-better".into();
        def.unit = Some("%".into());
        let id = store.upsert_definition(def.clone()).await.unwrap();

        // Same key updates in place (no new row).
        def.title = "Diff coverage (changed lines)".into();
        let id2 = store.upsert_definition(def).await.unwrap();
        assert_eq!(id, id2);

        let got = store
            .get_definition("oxplow.coverage.diff_pct")
            .await
            .unwrap()
            .expect("definition exists");
        assert_eq!(got.title, "Diff coverage (changed lines)");
        assert_eq!(got.direction, "higher-better");
        assert_eq!(got.unit.as_deref(), Some("%"));
        assert_eq!(store.list_definitions().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn run_with_samples_and_findings_round_trips() {
        let store = fixture().await;
        let metric = gauge_def(&store, "rust.unsafe_blocks").await;
        let run = store
            .record_run(NewMetricRun::done(1, "metrics", "builtin"))
            .await
            .unwrap();

        store
            .record_sample(NewMetricSample {
                run_id: Some(run),
                subject_kind: Some("tree".into()),
                subject_ref: Some(".".into()),
                ..NewMetricSample::observed(metric, 1, 3.0, "builtin")
            })
            .await
            .unwrap();
        store
            .record_finding(NewMetricFinding {
                run_id: run,
                metric_id: Some(metric),
                subject_kind: None,
                subject_ref: None,
                path: Some("src/a.rs".into()),
                start_line: Some(12),
                end_line: Some(12),
                col: None,
                kind: "lint".into(),
                severity: Some("warning".into()),
                rule: Some("unsafe-block".into()),
                message: Some("unsafe block".into()),
                value: None,
                extra_json: None,
            })
            .await
            .unwrap();

        let samples = store.list_samples(metric).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].value, 3.0);
        assert_eq!(samples[0].run_id, Some(run));
        assert_eq!(samples[0].subject_ref.as_deref(), Some("."));

        let findings = store.list_findings(run).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule.as_deref(), Some("unsafe-block"));
        assert_eq!(findings[0].path.as_deref(), Some("src/a.rs"));
    }

    #[tokio::test]
    async fn record_run_with_data_writes_atomically_and_backfills_run_id() {
        let store = fixture().await;
        let metric = gauge_def(&store, "rust.unsafe_blocks").await;
        // Samples/findings carry no run_id — the composite must backfill it.
        let s1 = NewMetricSample::observed(metric, 1, 1.0, "builtin");
        let s2 = NewMetricSample::observed(metric, 1, 2.0, "builtin");
        let f = NewMetricFinding {
            run_id: 0,
            metric_id: Some(metric),
            subject_kind: None,
            subject_ref: None,
            path: Some("src/a.rs".into()),
            start_line: Some(1),
            end_line: Some(1),
            col: None,
            kind: "lint".into(),
            severity: Some("warning".into()),
            rule: Some("r".into()),
            message: None,
            value: None,
            extra_json: None,
        };
        let run = store
            .record_run_with_data(NewMetricRun::done(1, "p", "builtin"), vec![s1, s2], vec![f])
            .await
            .unwrap();

        let samples = store.list_samples(metric).await.unwrap();
        assert_eq!(samples.len(), 2);
        // Every sample is stitched to the run id the composite returned.
        assert!(samples.iter().all(|s| s.run_id == Some(run)));
        let findings = store.list_findings(run).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].run_id, run);
    }

    #[tokio::test]
    async fn gc_effort_leaves_samples_untouched() {
        let store = fixture().await;
        let metric = gauge_def(&store, "loc").await;
        // Sample captured inside the effort window, but with NO effort FK.
        store
            .record_sample(NewMetricSample {
                captured_at: Some(at("2026-05-26T10:30:00Z")),
                ..NewMetricSample::observed(metric, 1, 1234.0, "builtin")
            })
            .await
            .unwrap();

        // Deleting the effort must not touch the durable sample.
        store
            .db
            .call(|conn| conn.execute("DELETE FROM task_effort WHERE id = 1", []))
            .await
            .unwrap();

        let samples = store.list_samples(metric).await.unwrap();
        assert_eq!(samples.len(), 1, "sample survives effort deletion (no FK)");
        assert_eq!(samples[0].value, 1234.0);
    }

    #[tokio::test]
    async fn samples_for_effort_uses_time_overlap() {
        let store = fixture().await;
        let metric = gauge_def(&store, "loc").await;
        // Window is [10:00, 11:00].
        for (ts, v) in [
            ("2026-05-26T09:30:00Z", 1.0), // before
            ("2026-05-26T10:30:00Z", 2.0), // inside
            ("2026-05-26T10:45:00Z", 3.0), // inside
            ("2026-05-26T11:30:00Z", 4.0), // after
        ] {
            store
                .record_sample(NewMetricSample {
                    captured_at: Some(at(ts)),
                    ..NewMetricSample::observed(metric, 1, v, "builtin")
                })
                .await
                .unwrap();
        }
        let in_effort = store.samples_for_effort(metric, 1).await.unwrap();
        let values: Vec<f64> = in_effort.iter().map(|s| s.value).collect();
        assert_eq!(
            values,
            vec![2.0, 3.0],
            "only in-window samples, time-ordered"
        );
    }

    #[tokio::test]
    async fn file_subject_samples_excluded_from_headline_reads() {
        let store = fixture().await;
        let metric = gauge_def(&store, "oxplow.rust.unsafe_blocks").await;
        // A headline `tree:.` total plus two per-file samples, all in the window.
        for s in [
            NewMetricSample {
                captured_at: Some(at("2026-05-26T10:30:00Z")),
                subject_kind: Some("tree".into()),
                subject_ref: Some(".".into()),
                ..NewMetricSample::observed(metric, 1, 5.0, "builtin")
            },
            NewMetricSample {
                captured_at: Some(at("2026-05-26T10:30:00Z")),
                subject_kind: Some("file".into()),
                subject_ref: Some("src/a.rs".into()),
                ..NewMetricSample::observed(metric, 1, 3.0, "builtin")
            },
            NewMetricSample {
                captured_at: Some(at("2026-05-26T10:30:00Z")),
                subject_kind: Some("file".into()),
                subject_ref: Some("src/b.rs".into()),
                ..NewMetricSample::observed(metric, 1, 2.0, "builtin")
            },
        ] {
            store.record_sample(s).await.unwrap();
        }
        // Headline reads see ONLY the tree total — the file rows are the rollup
        // grain, not the trend/summary series.
        let headline = store.list_samples(metric).await.unwrap();
        assert_eq!(headline.len(), 1);
        assert_eq!(headline[0].value, 5.0);
        let in_effort = store.samples_for_effort(metric, 1).await.unwrap();
        assert_eq!(
            in_effort.iter().map(|s| s.value).collect::<Vec<_>>(),
            vec![5.0]
        );
    }

    #[tokio::test]
    async fn file_samples_for_paths_scopes_by_stream_and_paths() {
        let store = fixture().await;
        let metric = gauge_def(&store, "oxplow.rust.unsafe_blocks").await;
        // A second stream so the stream scope is exercised (a file changed on two
        // worktrees has samples under two streams — they must not cross-leak).
        store
            .db
            .call(|conn| {
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES (2, 'worktree', 'w', 'feat', 'refs/heads/feat', 'feat', '/w', '2026-05-26T00:00:00Z', '2026-05-26T00:00:00Z')",
                    [],
                )
            })
            .await
            .unwrap();
        let rows = [
            (1_i64, "src/a.rs", "2026-05-26T10:00:00Z", 2.0),
            (1, "src/a.rs", "2026-05-26T10:40:00Z", 1.0),
            (1, "src/b.rs", "2026-05-26T10:40:00Z", 4.0),
            (1, "src/c.rs", "2026-05-26T10:40:00Z", 9.0), // path not requested
            (2, "src/a.rs", "2026-05-26T10:40:00Z", 99.0), // other stream
        ];
        for (stream, path, ts, v) in rows {
            store
                .record_sample(NewMetricSample {
                    captured_at: Some(at(ts)),
                    subject_kind: Some("file".into()),
                    subject_ref: Some(path.into()),
                    ..NewMetricSample::observed(metric, stream, v, "builtin")
                })
                .await
                .unwrap();
        }
        let got = store
            .file_samples_for_paths(metric, 1, vec!["src/a.rs".into(), "src/b.rs".into()])
            .await
            .unwrap();
        // Only stream-1 a.rs (both, oldest-first) + b.rs; c.rs and stream-2 out.
        let pairs: Vec<(String, f64)> = got
            .iter()
            .map(|s| (s.subject_ref.clone().unwrap(), s.value))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("src/a.rs".to_string(), 2.0),
                ("src/a.rs".to_string(), 1.0),
                ("src/b.rs".to_string(), 4.0),
            ]
        );
        // Empty paths short-circuits.
        assert!(store
            .file_samples_for_paths(metric, 1, vec![])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn sub_second_samples_window_and_order_correctly() {
        let store = fixture().await;
        let metric = gauge_def(&store, "loc").await;
        // Window is [10:00:00.000000, 11:00:00.000000]. Mix sub-second precisions
        // at and around the boundary + within one second — the cases that broke
        // under variable-width lexicographic comparison before canonicalization.
        for (ts, v) in [
            ("2026-05-26T09:59:59.999999Z", 1.0), // just before start → excluded
            ("2026-05-26T10:00:00Z", 2.0),        // exactly start (whole sec) → included
            ("2026-05-26T10:30:00.750Z", 3.0),    // inside
            ("2026-05-26T10:30:00Z", 4.0),        // inside, same second, whole
            ("2026-05-26T10:30:00.250Z", 5.0),    // inside, same second
            ("2026-05-26T11:00:00.000001Z", 6.0), // just after end → excluded
        ] {
            store
                .record_sample(NewMetricSample {
                    captured_at: Some(at(ts)),
                    ..NewMetricSample::observed(metric, 1, v, "builtin")
                })
                .await
                .unwrap();
        }
        // samples_for_effort is time-ASC: the boundary `10:00:00` is included,
        // the just-before/just-after are excluded, and the three same-second
        // samples sort whole < .250 < .750 (values 4, 5, 3).
        let in_effort = store.samples_for_effort(metric, 1).await.unwrap();
        let values: Vec<f64> = in_effort.iter().map(|s| s.value).collect();
        assert_eq!(values, vec![2.0, 4.0, 5.0, 3.0]);

        // list_samples is newest-first: reverse chronological across precisions.
        let newest = store.list_samples(metric).await.unwrap();
        let order: Vec<f64> = newest.iter().map(|s| s.value).collect();
        assert_eq!(order, vec![6.0, 3.0, 5.0, 4.0, 2.0, 1.0]);
    }

    #[tokio::test]
    async fn ratio_reaggregates_from_components_not_naive_average() {
        let store = fixture().await;
        let metric = gauge_def(&store, "coverage").await;
        // File A: 1/1 covered (100%). File B: 0/3 covered (0%).
        // Naive AVG of the two %s = 50%. True combined = (1+0)/(1+3) = 25%.
        store
            .record_sample(NewMetricSample {
                numerator: Some(1.0),
                denominator: Some(1.0),
                ..NewMetricSample::observed(metric, 1, 1.0, "builtin")
            })
            .await
            .unwrap();
        store
            .record_sample(NewMetricSample {
                numerator: Some(0.0),
                denominator: Some(3.0),
                ..NewMetricSample::observed(metric, 1, 0.0, "builtin")
            })
            .await
            .unwrap();

        let combined = store.aggregate_ratio(metric).await.unwrap().unwrap();
        assert!((combined - 0.25).abs() < 1e-9, "got {combined}");
        let naive_avg = 0.5_f64;
        assert!(
            (combined - naive_avg).abs() > 1e-6,
            "must differ from naive average"
        );
    }

    #[tokio::test]
    async fn branch_is_tracked_on_run_and_sample() {
        let store = fixture().await;
        let metric = gauge_def(&store, "loc").await;
        let run = store
            .record_run(NewMetricRun {
                branch: Some("metrics-substrate".into()),
                closest_git_version: Some("abc1234".into()),
                ..NewMetricRun::done(1, "metrics", "builtin")
            })
            .await
            .unwrap();
        store
            .record_sample(NewMetricSample {
                run_id: Some(run),
                branch: Some("metrics-substrate".into()),
                ..NewMetricSample::observed(metric, 1, 42.0, "builtin")
            })
            .await
            .unwrap();

        let got_run = store.get_run(run).await.unwrap().unwrap();
        assert_eq!(got_run.branch.as_deref(), Some("metrics-substrate"));
        let samples = store.list_samples(metric).await.unwrap();
        assert_eq!(samples[0].branch.as_deref(), Some("metrics-substrate"));

        // "When applicable": a branch-less sample stays None.
        store
            .record_sample(NewMetricSample::observed(metric, 1, 1.0, "builtin"))
            .await
            .unwrap();
        let all = store.list_samples(metric).await.unwrap();
        assert!(all.iter().any(|s| s.branch.is_none()));
    }
}
