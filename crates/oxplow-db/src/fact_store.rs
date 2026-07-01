//! Durable atomic FACT layer (epic tsk12, child tsk13) — the inverted metric
//! substrate. Backs `V43__metric_facts.sql`:
//!   * [`Measure`] — the catalog of fact TYPES (what a collector may emit).
//!   * [`Dimension`] — the conformed-dimension catalog (cross-metric drill-across).
//!   * [`MetricCapture`] — the ONE context row (renamed from `metric_run`): all
//!     when/where/who/effort/trust metadata lives here, once.
//!   * [`NewFact`] / [`FactRow`] — the durable atomic measurement (folds the V38
//!     `metric_sample` + `metric_finding`). A fact holds ONLY the measurement +
//!     subject + reported finding metadata + dims; its context is reached through
//!     `capture_id` (NOT NULL). [`FactRow`] is the joined read view.
//!
//! Built additively beside `metric_store.rs`; producers (tsk14) and reads (tsk16)
//! move onto it, then a cleanup migration drops the old tables. Modeled on
//! `metric_store.rs` (sync work inside `Database::call`, raw integer ids).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{canonical_ts, map_sql_err, Database};

// Timestamp <-> canonical string helpers (mirror `metric_store.rs`; kept local so
// the fact layer doesn't couple to the old module that will be deleted).
fn ts_to_string(ts: Timestamp) -> String {
    let raw = serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string();
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
// Measure (the catalog of fact types)
// ---------------------------------------------------------------------------

/// One row in the measure catalog — a kind of atomic fact a collector may emit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Measure {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    /// The grain's subject kind (`symbol` | `file` | `test` | `model` | …).
    pub subject_kind: Option<String>,
    /// `additive` | `semi-additive` | `non-additive` — additivity OVER TIME.
    pub temporal_semantics: String,
    /// `none` | `numerator` | `denominator` — ratio-base role.
    pub component_role: String,
    /// `built-in` | `global` | `project`.
    pub scope: String,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Write-side input for [`SqliteFactStore::upsert_measure`].
#[derive(Debug, Clone)]
pub struct NewMeasure {
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    pub subject_kind: Option<String>,
    pub temporal_semantics: String,
    pub component_role: String,
    pub scope: String,
    pub description: Option<String>,
}

impl NewMeasure {
    /// A `semi-additive`, `none`-role, `built-in` measure (snapshot-measure
    /// defaults — the common case for code metrics).
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            unit: None,
            subject_kind: None,
            temporal_semantics: "semi-additive".into(),
            component_role: "none".into(),
            scope: "built-in".into(),
            description: None,
        }
    }
}

const MEASURE_COLS: &str = "id, key, title, unit, subject_kind, temporal_semantics, \
     component_role, scope, description, created_at, updated_at";

fn row_to_measure(row: &rusqlite::Row<'_>) -> rusqlite::Result<Measure> {
    let created_at: String = row.get(9)?;
    let updated_at: String = row.get(10)?;
    Ok(Measure {
        id: row.get(0)?,
        key: row.get(1)?,
        title: row.get(2)?,
        unit: row.get(3)?,
        subject_kind: row.get(4)?,
        temporal_semantics: row.get(5)?,
        component_role: row.get(6)?,
        scope: row.get(7)?,
        description: row.get(8)?,
        created_at: string_to_ts(&created_at).map_err(ts_conv_err)?,
        updated_at: string_to_ts(&updated_at).map_err(ts_conv_err)?,
    })
}

// ---------------------------------------------------------------------------
// Dimension (the conformed-dimension catalog)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Dimension {
    pub key: String,
    pub label: String,
    /// `categorical` | `numeric` | `temporal` | `entity-ref`.
    pub value_type: String,
    pub subject_kind: Option<String>,
    pub vocabulary_json: Option<String>,
    pub scope: String,
    /// Whether a generated column + expression index exists on `fact` for this dim.
    pub promoted: bool,
}

#[derive(Debug, Clone)]
pub struct NewDimension {
    pub key: String,
    pub label: String,
    pub value_type: String,
    pub subject_kind: Option<String>,
    pub vocabulary_json: Option<String>,
    pub scope: String,
}

impl NewDimension {
    /// A `categorical`, `built-in` dimension.
    pub fn categorical(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: "categorical".into(),
            subject_kind: None,
            vocabulary_json: None,
            scope: "built-in".into(),
        }
    }
}

const DIM_COLS: &str = "key, label, value_type, subject_kind, vocabulary_json, scope, promoted";

fn row_to_dimension(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dimension> {
    Ok(Dimension {
        key: row.get(0)?,
        label: row.get(1)?,
        value_type: row.get(2)?,
        subject_kind: row.get(3)?,
        vocabulary_json: row.get(4)?,
        scope: row.get(5)?,
        promoted: row.get::<_, i64>(6)? != 0,
    })
}

// ---------------------------------------------------------------------------
// MetricSpec (the metric-as-a-spec catalog — the third catalog beside measure
// + dimension). A metric is NOT a stored sample stream; it is a spec computed
// over facts: a source measure + an aggregation + an optional filter/formula,
// plus read-time presentation (direction + thresholds + display kind).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricSpec {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    /// The measure whose facts this metric aggregates; `None` for a formula metric.
    pub source_measure: Option<String>,
    /// `count` | `count_distinct` | `sum` | `avg` | `min` | `max` | `last` | `p95`
    /// | `ratio` — how source facts combine WITHIN a capture.
    pub aggregation: String,
    /// Conjunctive fact predicate (min_value / severity / dim equality), JSON.
    pub filter_json: Option<String>,
    /// Derived-metric formula referencing other metric keys; `None` for a base.
    pub formula: Option<String>,
    /// Conformed dims this metric may be sliced by (JSON array of dim keys).
    pub sliceable_dims_json: Option<String>,
    /// `higher-better` | `lower-better` | `neutral`.
    pub direction: String,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    /// `built-in` | `global` | `project`.
    pub scope: String,
    /// Read-time presentation: `gauge` | `findings` | `test` | `coverage` | `event`.
    pub display_kind: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Write-side input for [`SqliteFactStore::upsert_spec`]. Build with
/// [`NewMetricSpec::base`] then override fields.
#[derive(Debug, Clone)]
pub struct NewMetricSpec {
    pub key: String,
    pub title: String,
    pub unit: Option<String>,
    pub source_measure: Option<String>,
    pub aggregation: String,
    pub filter_json: Option<String>,
    pub formula: Option<String>,
    pub sliceable_dims_json: Option<String>,
    pub direction: String,
    pub target: Option<f64>,
    pub warn_at: Option<f64>,
    pub fail_at: Option<f64>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub language: Option<String>,
    pub scope: String,
    pub display_kind: String,
}

impl NewMetricSpec {
    /// A base metric over `source_measure` with `aggregation`; neutral direction,
    /// `gauge` display, `built-in` scope. For a formula metric, set
    /// `source_measure = None` and populate `formula`.
    pub fn base(
        key: impl Into<String>,
        title: impl Into<String>,
        source_measure: impl Into<String>,
        aggregation: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            unit: None,
            source_measure: Some(source_measure.into()),
            aggregation: aggregation.into(),
            filter_json: None,
            formula: None,
            sliceable_dims_json: None,
            direction: "neutral".into(),
            target: None,
            warn_at: None,
            fail_at: None,
            description: None,
            category: None,
            language: None,
            scope: "built-in".into(),
            display_kind: "gauge".into(),
        }
    }
}

const SPEC_COLS: &str = "id, key, title, unit, source_measure, aggregation, filter_json, \
     formula, sliceable_dims_json, direction, target, warn_at, fail_at, description, \
     category, language, scope, display_kind, created_at, updated_at";

fn row_to_spec(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricSpec> {
    let created_at: String = row.get(18)?;
    let updated_at: String = row.get(19)?;
    Ok(MetricSpec {
        id: row.get(0)?,
        key: row.get(1)?,
        title: row.get(2)?,
        unit: row.get(3)?,
        source_measure: row.get(4)?,
        aggregation: row.get(5)?,
        filter_json: row.get(6)?,
        formula: row.get(7)?,
        sliceable_dims_json: row.get(8)?,
        direction: row.get(9)?,
        target: row.get(10)?,
        warn_at: row.get(11)?,
        fail_at: row.get(12)?,
        description: row.get(13)?,
        category: row.get(14)?,
        language: row.get(15)?,
        scope: row.get(16)?,
        display_kind: row.get(17)?,
        created_at: string_to_ts(&created_at).map_err(ts_conv_err)?,
        updated_at: string_to_ts(&updated_at).map_err(ts_conv_err)?,
    })
}

// ---------------------------------------------------------------------------
// Capture (the context row)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricCapture {
    pub id: i64,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    /// The producing effort (provenance), nullable; SET NULL on effort GC.
    pub effort_id: Option<i64>,
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
    pub git_version_exact: bool,
    pub branch: Option<String>,
    pub captured_at: Timestamp,
    pub ended_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct NewMetricCapture {
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub effort_id: Option<i64>,
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
    pub git_version_exact: bool,
    pub branch: Option<String>,
    /// Defaults to now when `None`.
    pub captured_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
}

impl NewMetricCapture {
    /// A completed (`status = done`, `provenance = observed`) capture.
    pub fn done(stream_id: i64, producer: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            stream_id,
            thread_id: None,
            effort_id: None,
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
            git_version_exact: false,
            branch: None,
            captured_at: None,
            ended_at: None,
        }
    }
}

const CAPTURE_COLS: &str = "id, stream_id, thread_id, effort_id, producer, status, error, scope, \
     trigger, basis_ref, provenance, source, snapshot_id, closest_git_version, git_version_exact, \
     branch, captured_at, ended_at";

fn row_to_capture(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetricCapture> {
    let captured_at: String = row.get(16)?;
    let ended_at: Option<String> = row.get(17)?;
    Ok(MetricCapture {
        id: row.get(0)?,
        stream_id: row.get(1)?,
        thread_id: row.get(2)?,
        effort_id: row.get(3)?,
        producer: row.get(4)?,
        status: row.get(5)?,
        error: row.get(6)?,
        scope: row.get(7)?,
        trigger: row.get(8)?,
        basis_ref: row.get(9)?,
        provenance: row.get(10)?,
        source: row.get(11)?,
        snapshot_id: row.get(12)?,
        closest_git_version: row.get(13)?,
        git_version_exact: row.get::<_, i64>(14)? != 0,
        branch: row.get(15)?,
        captured_at: string_to_ts(&captured_at).map_err(ts_conv_err)?,
        ended_at: match ended_at {
            Some(s) => Some(string_to_ts(&s).map_err(ts_conv_err)?),
            None => None,
        },
    })
}

fn insert_capture(conn: &rusqlite::Connection, c: NewMetricCapture) -> rusqlite::Result<i64> {
    let captured = c
        .captured_at
        .map(ts_to_string)
        .unwrap_or_else(|| ts_to_string(Timestamp::now()));
    let ended = c.ended_at.map(ts_to_string);
    conn.execute(
        "INSERT INTO metric_capture
           (stream_id, thread_id, effort_id, producer, status, error, scope, trigger, basis_ref,
            provenance, source, snapshot_id, closest_git_version, git_version_exact, branch,
            captured_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            c.stream_id,
            c.thread_id,
            c.effort_id,
            c.producer,
            c.status,
            c.error,
            c.scope,
            c.trigger,
            c.basis_ref,
            c.provenance,
            c.source,
            c.snapshot_id,
            c.closest_git_version,
            c.git_version_exact,
            c.branch,
            captured,
            ended,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------------------------------------------------------------------------
// Fact (the durable atomic measurement)
// ---------------------------------------------------------------------------

/// Write-side input for a single fact. `capture_id` is backfilled by
/// [`SqliteFactStore::record_facts`]; set it directly only via `record_fact`.
#[derive(Debug, Clone)]
pub struct NewFact {
    pub capture_id: Option<i64>,
    pub measure_id: i64,
    pub value: f64,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub severity: Option<String>,
    pub rule: Option<String>,
    pub detail: Option<String>,
    pub dims_json: Option<String>,
}

impl NewFact {
    /// A minimal fact of `measure_id` with `value` (capture backfilled).
    pub fn new(measure_id: i64, value: f64) -> Self {
        Self {
            capture_id: None,
            measure_id,
            value,
            numerator: None,
            denominator: None,
            subject_kind: None,
            subject_ref: None,
            path: None,
            line: None,
            severity: None,
            rule: None,
            detail: None,
            dims_json: None,
        }
    }
}

/// The joined read view of a fact: its own measurement columns PLUS the spine it
/// inherits from its capture (`captured_at`, `branch`, version, effort, trust).
/// This is what the aggregation engine and reads consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FactRow {
    pub id: i64,
    pub capture_id: i64,
    pub measure_id: i64,
    pub value: f64,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub severity: Option<String>,
    pub rule: Option<String>,
    pub detail: Option<String>,
    pub dims_json: Option<String>,
    // --- spine, inherited from the capture ---
    pub captured_at: Timestamp,
    pub branch: Option<String>,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
    pub basis_ref: Option<String>,
    pub snapshot_id: Option<i64>,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub effort_id: Option<i64>,
    pub provenance: String,
    pub source: String,
}

const FACT_ROW_COLS: &str = "f.id, f.capture_id, f.measure_id, f.value, f.numerator, \
     f.denominator, f.subject_kind, f.subject_ref, f.path, f.line, f.severity, f.rule, \
     f.detail, f.dims_json, c.captured_at, c.branch, c.closest_git_version, \
     c.git_version_exact, c.basis_ref, c.snapshot_id, c.stream_id, c.thread_id, \
     c.effort_id, c.provenance, c.source";

fn row_to_fact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FactRow> {
    let captured_at: String = row.get(14)?;
    Ok(FactRow {
        id: row.get(0)?,
        capture_id: row.get(1)?,
        measure_id: row.get(2)?,
        value: row.get(3)?,
        numerator: row.get(4)?,
        denominator: row.get(5)?,
        subject_kind: row.get(6)?,
        subject_ref: row.get(7)?,
        path: row.get(8)?,
        line: row.get(9)?,
        severity: row.get(10)?,
        rule: row.get(11)?,
        detail: row.get(12)?,
        dims_json: row.get(13)?,
        captured_at: string_to_ts(&captured_at).map_err(ts_conv_err)?,
        branch: row.get(15)?,
        closest_git_version: row.get(16)?,
        git_version_exact: row.get::<_, i64>(17)? != 0,
        basis_ref: row.get(18)?,
        snapshot_id: row.get(19)?,
        stream_id: row.get(20)?,
        thread_id: row.get(21)?,
        effort_id: row.get(22)?,
        provenance: row.get(23)?,
        source: row.get(24)?,
    })
}

fn insert_fact(conn: &rusqlite::Connection, f: NewFact) -> rusqlite::Result<i64> {
    let capture_id = f.capture_id.expect("fact must carry a capture_id");
    conn.execute(
        "INSERT INTO fact
           (capture_id, measure_id, value, numerator, denominator, subject_kind, subject_ref,
            path, line, severity, rule, detail, dims_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            capture_id,
            f.measure_id,
            f.value,
            f.numerator,
            f.denominator,
            f.subject_kind,
            f.subject_ref,
            f.path,
            f.line,
            f.severity,
            f.rule,
            f.detail,
            f.dims_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SqliteFactStore {
    db: Database,
}

impl SqliteFactStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    // --- catalogs ---------------------------------------------------------

    /// Insert or update (by `key`) a measure; returns its row id. `created_at` is
    /// preserved across updates.
    pub async fn upsert_measure(&self, m: NewMeasure) -> Result<i64, DomainError> {
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO measure
                       (key, title, unit, subject_kind, temporal_semantics, component_role, scope,
                        description, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                     ON CONFLICT(key) DO UPDATE SET
                        title=excluded.title, unit=excluded.unit,
                        subject_kind=excluded.subject_kind,
                        temporal_semantics=excluded.temporal_semantics,
                        component_role=excluded.component_role, scope=excluded.scope,
                        description=excluded.description, updated_at=excluded.updated_at",
                    params![
                        m.key,
                        m.title,
                        m.unit,
                        m.subject_kind,
                        m.temporal_semantics,
                        m.component_role,
                        m.scope,
                        m.description,
                        now,
                    ],
                )?;
                conn.query_row(
                    "SELECT id FROM measure WHERE key = ?1",
                    params![m.key],
                    |r| r.get(0),
                )
            })
            .await
    }

    pub async fn get_measure(&self, key: &str) -> Result<Option<Measure>, DomainError> {
        let key = key.to_string();
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {MEASURE_COLS} FROM measure WHERE key = ?1");
                conn.query_row(&sql, params![key], row_to_measure)
                    .optional()
            })
            .await
    }

    pub async fn list_measures(&self) -> Result<Vec<Measure>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {MEASURE_COLS} FROM measure ORDER BY key");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_measure)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Insert or update (by `key`) a dimension in the conformed catalog.
    pub async fn upsert_dimension(&self, d: NewDimension) -> Result<(), DomainError> {
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO dimension (key, label, value_type, subject_kind, vocabulary_json, scope)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(key) DO UPDATE SET
                        label=excluded.label, value_type=excluded.value_type,
                        subject_kind=excluded.subject_kind,
                        vocabulary_json=excluded.vocabulary_json, scope=excluded.scope",
                    params![d.key, d.label, d.value_type, d.subject_kind, d.vocabulary_json, d.scope],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_dimensions(&self) -> Result<Vec<Dimension>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {DIM_COLS} FROM dimension ORDER BY key");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_dimension)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Insert or update (by `key`) a metric spec; returns its row id. `created_at`
    /// is preserved across updates.
    pub async fn upsert_spec(&self, s: NewMetricSpec) -> Result<i64, DomainError> {
        self.db
            .call(move |conn| {
                let now = ts_to_string(Timestamp::now());
                conn.execute(
                    "INSERT INTO metric_spec
                       (key, title, unit, source_measure, aggregation, filter_json, formula,
                        sliceable_dims_json, direction, target, warn_at, fail_at, description,
                        category, language, scope, display_kind, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16, ?17, ?18, ?18)
                     ON CONFLICT(key) DO UPDATE SET
                        title=excluded.title, unit=excluded.unit,
                        source_measure=excluded.source_measure, aggregation=excluded.aggregation,
                        filter_json=excluded.filter_json, formula=excluded.formula,
                        sliceable_dims_json=excluded.sliceable_dims_json,
                        direction=excluded.direction, target=excluded.target,
                        warn_at=excluded.warn_at, fail_at=excluded.fail_at,
                        description=excluded.description, category=excluded.category,
                        language=excluded.language, scope=excluded.scope,
                        display_kind=excluded.display_kind, updated_at=excluded.updated_at",
                    params![
                        s.key,
                        s.title,
                        s.unit,
                        s.source_measure,
                        s.aggregation,
                        s.filter_json,
                        s.formula,
                        s.sliceable_dims_json,
                        s.direction,
                        s.target,
                        s.warn_at,
                        s.fail_at,
                        s.description,
                        s.category,
                        s.language,
                        s.scope,
                        s.display_kind,
                        now,
                    ],
                )?;
                conn.query_row(
                    "SELECT id FROM metric_spec WHERE key = ?1",
                    params![s.key],
                    |r| r.get(0),
                )
            })
            .await
    }

    pub async fn get_spec(&self, key: &str) -> Result<Option<MetricSpec>, DomainError> {
        let key = key.to_string();
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {SPEC_COLS} FROM metric_spec WHERE key = ?1");
                conn.query_row(&sql, params![key], row_to_spec).optional()
            })
            .await
    }

    pub async fn list_specs(&self) -> Result<Vec<MetricSpec>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {SPEC_COLS} FROM metric_spec ORDER BY key");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_spec)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    // --- captures + facts -------------------------------------------------

    /// Insert one capture; returns its id. `captured_at` defaults to now.
    pub async fn record_capture(&self, c: NewMetricCapture) -> Result<i64, DomainError> {
        self.db.call(move |conn| insert_capture(conn, c)).await
    }

    /// Atomically insert a capture plus all of its facts in one transaction. Each
    /// fact's `capture_id` is forced to the new capture's id, so a producer can't
    /// leave a half-written graph behind on a crash mid-write. Returns the
    /// capture id.
    pub async fn record_facts(
        &self,
        capture: NewMetricCapture,
        facts: Vec<NewFact>,
    ) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let capture_id = insert_capture(&tx, capture).map_err(map_sql_err)?;
                for mut f in facts {
                    f.capture_id = Some(capture_id);
                    insert_fact(&tx, f).map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(capture_id)
            })
            .await
    }

    pub async fn get_capture(&self, capture_id: i64) -> Result<Option<MetricCapture>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {CAPTURE_COLS} FROM metric_capture WHERE id = ?1");
                conn.query_row(&sql, params![capture_id], row_to_capture)
                    .optional()
            })
            .await
    }

    /// All facts of a measure, joined to their capture for the spine, oldest
    /// capture first.
    pub async fn facts_for_measure(&self, measure_id: i64) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.measure_id = ?1
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![measure_id], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Facts of a measure belonging to the given captures (the attribution-by-claim
    /// read — an effort's facts are those of its claimed captures, not a time
    /// window). Oldest-first. Empty when `capture_ids` is empty.
    pub async fn facts_for_captures(
        &self,
        measure_id: i64,
        capture_ids: Vec<i64>,
    ) -> Result<Vec<FactRow>, DomainError> {
        if capture_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |conn| {
                let placeholders = std::iter::repeat("?")
                    .take(capture_ids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.measure_id = ? AND f.capture_id IN ({placeholders})
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&measure_id];
                for id in &capture_ids {
                    binds.push(id);
                }
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Correctly aggregate a ratio measure by re-combining stored components:
    /// `sum(numerator) / sum(denominator)`. Returns `None` when no fact carries
    /// components or the denominators sum to zero — this is what makes "coverage %
    /// by module" right instead of a naive average of per-file %s.
    pub async fn aggregate_ratio(&self, measure_id: i64) -> Result<Option<f64>, DomainError> {
        self.db
            .call(move |conn| {
                let row: Option<(Option<f64>, Option<f64>)> = conn
                    .query_row(
                        "SELECT SUM(numerator), SUM(denominator) FROM fact
                          WHERE measure_id = ?1 AND numerator IS NOT NULL AND denominator IS NOT NULL",
                        params![measure_id],
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

    /// stream(1) + thread(1) + task + effort so capture FKs resolve and the
    /// effort-GC test has a real effort to delete.
    async fn fixture() -> SqliteFactStore {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-06-30T00:00:00Z";
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
                conn.execute(
                    "INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at)
                     VALUES (1, ?1, 1, '2026-06-30T10:00:00.000000Z', '2026-06-30T11:00:00.000000Z')",
                    params![task_id],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        SqliteFactStore::new(db)
    }

    async fn measure(store: &SqliteFactStore, key: &str) -> i64 {
        store
            .upsert_measure(NewMeasure::new(key, key))
            .await
            .unwrap()
    }

    fn at(ts: &str) -> Timestamp {
        string_to_ts(ts).unwrap()
    }

    #[tokio::test]
    async fn upsert_measure_inserts_then_updates_in_place() {
        let store = fixture().await;
        // A non-seeded key so the insert adds a new row (the migration already
        // seeds the `oxplow.*` built-ins).
        let mut m = NewMeasure::new("acme.api_latency", "API latency");
        m.unit = Some("ms".into());
        m.subject_kind = Some("endpoint".into());
        let id = store.upsert_measure(m.clone()).await.unwrap();

        m.title = "API latency (p95)".into();
        let id2 = store.upsert_measure(m).await.unwrap();
        assert_eq!(id, id2, "same key updates in place");

        let got = store
            .get_measure("acme.api_latency")
            .await
            .unwrap()
            .expect("measure exists");
        assert_eq!(got.title, "API latency (p95)");
        assert_eq!(got.subject_kind.as_deref(), Some("endpoint"));
        assert_eq!(got.temporal_semantics, "semi-additive");
        // The migrations seed 11 built-in measures (10 in V43 + oxplow.ast_hit in
        // V45); this upsert added one more.
        assert_eq!(store.list_measures().await.unwrap().len(), 12);
    }

    #[tokio::test]
    async fn dimensions_seeded_and_upsertable() {
        let store = fixture().await;
        // The migrations seed 9 built-in conformed dims (8 in V43 + oxplow.rule
        // in V45).
        let seeded = store.list_dimensions().await.unwrap();
        assert!(seeded.iter().any(|d| d.key == "oxplow.language"));
        assert!(seeded.iter().any(|d| d.key == "oxplow.rule"));
        assert_eq!(seeded.len(), 9);

        store
            .upsert_dimension(NewDimension {
                scope: "project".into(),
                ..NewDimension::categorical("acme.license", "License")
            })
            .await
            .unwrap();
        let after = store.list_dimensions().await.unwrap();
        let lic = after
            .iter()
            .find(|d| d.key == "acme.license")
            .expect("custom dim registered");
        assert_eq!(lic.scope, "project");
        assert!(!lic.promoted);
    }

    #[tokio::test]
    async fn upsert_spec_round_trips_and_updates_in_place() {
        let store = fixture().await;
        // The migration seeds NO specs — this is the first row.
        assert!(store.list_specs().await.unwrap().is_empty());

        let mut s = NewMetricSpec::base(
            "acme.hotspots",
            "Complexity hotspots",
            "oxplow.complexity",
            "count",
        );
        s.unit = Some("count".into());
        s.filter_json = Some("{\"min_value\":10.0}".into());
        s.sliceable_dims_json = Some("[\"oxplow.package\"]".into());
        s.direction = "lower-better".into();
        s.warn_at = Some(5.0);
        s.fail_at = Some(10.0);
        s.scope = "project".into();
        s.display_kind = "gauge".into();
        let id = store.upsert_spec(s.clone()).await.unwrap();

        // Same key updates in place, preserving the id.
        let mut s2 = s.clone();
        s2.title = "Hotspots (\u{2265}10)".into();
        let id2 = store.upsert_spec(s2).await.unwrap();
        assert_eq!(id, id2, "same key updates in place");

        let got = store
            .get_spec("acme.hotspots")
            .await
            .unwrap()
            .expect("spec exists");
        assert_eq!(got.title, "Hotspots (\u{2265}10)");
        assert_eq!(got.source_measure.as_deref(), Some("oxplow.complexity"));
        assert_eq!(got.aggregation, "count");
        assert_eq!(got.filter_json.as_deref(), Some("{\"min_value\":10.0}"));
        assert_eq!(
            got.sliceable_dims_json.as_deref(),
            Some("[\"oxplow.package\"]")
        );
        assert_eq!(got.direction, "lower-better");
        assert_eq!(got.warn_at, Some(5.0));
        assert_eq!(got.fail_at, Some(10.0));
        assert_eq!(got.scope, "project");
        assert_eq!(got.display_kind, "gauge");

        let all = store.list_specs().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key, "acme.hotspots");
    }

    #[tokio::test]
    async fn upsert_spec_allows_formula_metric_without_source_measure() {
        let store = fixture().await;
        // A derived (formula) metric has no source measure.
        let mut s = NewMetricSpec::base("acme.bugs_per_kloc", "Bugs per KLOC", "", "ratio");
        s.source_measure = None;
        s.formula = Some("{\"op\":\"div\",\"left\":\"acme.bugs\",\"right\":\"acme.kloc\"}".into());
        store.upsert_spec(s).await.unwrap();

        let got = store.get_spec("acme.bugs_per_kloc").await.unwrap().unwrap();
        assert!(got.source_measure.is_none());
        assert_eq!(
            got.formula.as_deref(),
            Some("{\"op\":\"div\",\"left\":\"acme.bugs\",\"right\":\"acme.kloc\"}")
        );
    }

    #[tokio::test]
    async fn record_facts_writes_atomically_and_backfills_capture_id() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        // Facts carry no capture_id — record_facts must backfill it.
        let facts = vec![
            NewFact {
                subject_kind: Some("symbol".into()),
                subject_ref: Some("src/a.rs::foo".into()),
                path: Some("src/a.rs".into()),
                line: Some(10),
                ..NewFact::new(m, 14.0)
            },
            NewFact {
                subject_kind: Some("symbol".into()),
                subject_ref: Some("src/a.rs::bar".into()),
                path: Some("src/a.rs".into()),
                line: Some(40),
                ..NewFact::new(m, 3.0)
            },
        ];
        let capture = store
            .record_facts(
                NewMetricCapture {
                    branch: Some("main".into()),
                    closest_git_version: Some("abc1234".into()),
                    ..NewMetricCapture::done(1, "metrics", "builtin")
                },
                facts,
            )
            .await
            .unwrap();

        let rows = store.facts_for_measure(m).await.unwrap();
        assert_eq!(rows.len(), 2);
        // Every fact is stitched to the capture, and inherits its spine.
        assert!(rows.iter().all(|f| f.capture_id == capture));
        assert!(rows.iter().all(|f| f.branch.as_deref() == Some("main")));
        assert!(rows
            .iter()
            .all(|f| f.closest_git_version.as_deref() == Some("abc1234")));
        // Oldest-first within the capture is by fact id (insertion order).
        assert_eq!(rows[0].subject_ref.as_deref(), Some("src/a.rs::foo"));
        assert_eq!(rows[0].value, 14.0);
        assert_eq!(rows[1].value, 3.0);
    }

    #[tokio::test]
    async fn effort_gc_nulls_capture_effort_but_keeps_facts() {
        // The core invariant: facts (and their capture) outlive the effort. GC of
        // the effort SET-NULLs the capture's effort_id, never deletes a fact.
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        let capture = store
            .record_facts(
                NewMetricCapture {
                    effort_id: Some(1),
                    captured_at: Some(at("2026-06-30T10:30:00Z")),
                    ..NewMetricCapture::done(1, "metrics", "builtin")
                },
                vec![NewFact::new(m, 7.0)],
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_capture(capture).await.unwrap().unwrap().effort_id,
            Some(1)
        );

        // Delete the effort — the capture and its fact must survive.
        store
            .db
            .call(|conn| conn.execute("DELETE FROM task_effort WHERE id = 1", []))
            .await
            .unwrap();

        let cap = store.get_capture(capture).await.unwrap().unwrap();
        assert_eq!(cap.effort_id, None, "effort_id SET NULL on GC");
        let rows = store.facts_for_measure(m).await.unwrap();
        assert_eq!(rows.len(), 1, "fact survives effort deletion");
        assert_eq!(rows[0].value, 7.0);
        assert_eq!(rows[0].effort_id, None);
    }

    #[tokio::test]
    async fn facts_for_captures_scopes_to_claimed_captures() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.test_case").await;
        let cap_a = store
            .record_facts(
                NewMetricCapture::done(1, "tests", "junit"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        let cap_b = store
            .record_facts(
                NewMetricCapture::done(1, "tests", "junit"),
                vec![NewFact::new(m, 2.0)],
            )
            .await
            .unwrap();

        // Only the claimed capture's facts come back.
        let only_a = store.facts_for_captures(m, vec![cap_a]).await.unwrap();
        assert_eq!(
            only_a.iter().map(|f| f.value).collect::<Vec<_>>(),
            vec![1.0]
        );
        let both = store
            .facts_for_captures(m, vec![cap_a, cap_b])
            .await
            .unwrap();
        assert_eq!(both.len(), 2);
        // Empty short-circuits.
        assert!(store
            .facts_for_captures(m, vec![])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn ratio_reaggregates_from_components_not_naive_average() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.coverage").await;
        // File A: 1/1 covered (100%). File B: 0/3 covered (0%).
        // Naive AVG of the two %s = 50%. True combined = (1+0)/(1+3) = 25%.
        store
            .record_facts(
                NewMetricCapture::done(1, "coverage", "lcov"),
                vec![
                    NewFact {
                        numerator: Some(1.0),
                        denominator: Some(1.0),
                        subject_kind: Some("file".into()),
                        subject_ref: Some("a.rs".into()),
                        ..NewFact::new(m, 1.0)
                    },
                    NewFact {
                        numerator: Some(0.0),
                        denominator: Some(3.0),
                        subject_kind: Some("file".into()),
                        subject_ref: Some("b.rs".into()),
                        ..NewFact::new(m, 0.0)
                    },
                ],
            )
            .await
            .unwrap();

        let combined = store.aggregate_ratio(m).await.unwrap().unwrap();
        assert!((combined - 0.25).abs() < 1e-9, "got {combined}");
        assert!(
            (combined - 0.5).abs() > 1e-6,
            "must differ from the naive average of per-file %s"
        );
    }

    #[tokio::test]
    async fn captures_durable_and_branchless_facts_allowed() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        // A branch-less capture (detached HEAD / non-git) stays None.
        let capture = store
            .record_facts(
                NewMetricCapture::done(1, "metrics", "builtin"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        let cap = store.get_capture(capture).await.unwrap().unwrap();
        assert_eq!(cap.branch, None);
        assert_eq!(cap.provenance, "observed");
        let rows = store.facts_for_measure(m).await.unwrap();
        assert!(rows[0].branch.is_none());
    }
}
