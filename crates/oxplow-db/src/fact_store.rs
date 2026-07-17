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
    /// `complete` | `per-path` | `per-subject` (V54 tsk41; per-subject V55
    /// tsk43) — what ONE capture restates. This is a SEPARATE AXIS from
    /// `temporal_semantics`: `complete` means a capture restates the whole
    /// population (a coverage report, an analysis run), so the temporal fold
    /// applies as-is; `per-path` restates only the paths in its snapshot (a
    /// tree gauge over a delta); `per-subject` restates only the subjects it
    /// emitted facts for (a test run — V55 moved test runs here). Partial
    /// scopes fold to the latest capture per key first — see
    /// `latest_tree_facts` / `latest_subject_facts`.
    pub capture_scope: String,
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
    /// `complete` | `per-path` — see [`Measure::capture_scope`].
    pub capture_scope: String,
    pub scope: String,
    pub description: Option<String>,
}

impl NewMeasure {
    /// A `semi-additive`, `complete`, `built-in` measure (snapshot-measure defaults
    /// — the common case for code metrics).
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            unit: None,
            subject_kind: None,
            temporal_semantics: "semi-additive".into(),
            capture_scope: "complete".into(),
            scope: "built-in".into(),
            description: None,
        }
    }
}

// `measure.component_role` (a dead V43 column, tsk15) is intentionally omitted
// from the read cols + upsert: it's never read, can't be safely `DROP COLUMN`d
// (a CHECK + the fact→measure CASCADE), and defaults to 'none' on insert.
const MEASURE_COLS: &str = "id, key, title, unit, subject_kind, temporal_semantics, \
     capture_scope, scope, description, created_at, updated_at";

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
        capture_scope: row.get(6)?,
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
    /// This dim is part of the aggregate cube's GRAIN (`metric_cube.dims_key`
    /// buckets by every promoted dim a fact carries — V62/V64). Flipping it on
    /// is a cube REBUILD, gated on measured cardinality; see
    /// `.context/metrics.md`.
    pub promoted: bool,
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
            promoted: false,
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
    /// The verbatim per-run detail payload, as an envelope
    /// `{"kind": "<detail kind>", "payload": {…}}` (test suite/case tree,
    /// coverage per-file line-sets, analysis findings) — the capture-spine home
    /// of the legacy `metric_finding` `*-detail` rows (T-E1, tsk48).
    pub detail_json: Option<String>,
    /// Fingerprint of the LOGIC that produced this capture (V56, tsk45) — for a
    /// gauge, a hash of its script + compute knobs + `emits`. When a gauge's current
    /// fingerprint no longer matches its latest capture's, its facts were computed by
    /// different logic and are stale, so a re-baseline is due. `None` = unversioned
    /// (pre-V56 rows, and producers whose logic isn't script-defined).
    pub producer_version: Option<String>,
    /// How this capture's SCANNED SET is determined (V58, tsk71):
    /// `delta` — the snapshot's own file rows (incremental rescan);
    /// `full` — the reconstructed tree as-of the snapshot (a baseline);
    /// `asserted` — exactly the paths it emitted facts for (a snapshot, when
    /// present, is provenance only). See the V58 migration header.
    pub scan_kind: String,
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
    /// See [`MetricCapture::detail_json`].
    pub detail_json: Option<String>,
    /// Optional CONTENT IDENTITY for idempotent ingestion (tsk14): a hash of
    /// producer + basis + verbatim payload. When set and a capture with the
    /// same key already exists, [`SqliteFactStore::record_facts`] skips the
    /// whole write (no duplicate capture, no double-counted facts) and returns
    /// the existing id. `None` (the default) always inserts a fresh row.
    pub idempotency_key: Option<String>,
    /// See [`MetricCapture::producer_version`].
    pub producer_version: Option<String>,
    /// See [`MetricCapture::scan_kind`]. Defaults to `delta`; the insert
    /// coerces a snapshot-less `delta` to `asserted` (delta/full semantics
    /// REQUIRE a snapshot to anchor their scanned set on).
    pub scan_kind: String,
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
            detail_json: None,
            idempotency_key: None,
            producer_version: None,
            scan_kind: "delta".into(),
        }
    }
}

const CAPTURE_COLS: &str = "id, stream_id, thread_id, effort_id, producer, status, error, scope, \
     trigger, basis_ref, provenance, source, snapshot_id, closest_git_version, git_version_exact, \
     branch, captured_at, ended_at, detail_json, producer_version, scan_kind";

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
        detail_json: row.get(18)?,
        producer_version: row.get(19)?,
        scan_kind: row.get(20)?,
    })
}

fn insert_capture(conn: &rusqlite::Connection, c: NewMetricCapture) -> rusqlite::Result<i64> {
    let captured = c
        .captured_at
        .map(ts_to_string)
        .unwrap_or_else(|| ts_to_string(Timestamp::now()));
    let ended = c.ended_at.map(ts_to_string);
    // `delta`/`full` scan semantics anchor on a snapshot's file rows; without a
    // snapshot there is nothing to anchor on, so the capture can only restate
    // the paths it emits — i.e. it IS an assertion. Coerce rather than trust
    // every caller to remember (the invariant the fold depends on).
    let scan_kind = if c.snapshot_id.is_none() && c.scan_kind != "asserted" {
        "asserted".to_string()
    } else {
        c.scan_kind.clone()
    };
    conn.execute(
        "INSERT INTO metric_capture
           (stream_id, thread_id, effort_id, producer, status, error, scope, trigger, basis_ref,
            provenance, source, snapshot_id, closest_git_version, git_version_exact, branch,
            captured_at, ended_at, detail_json, idempotency_key, producer_version, scan_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
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
            c.detail_json,
            c.idempotency_key,
            c.producer_version,
            scan_kind,
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

/// One `metric_cube` bucket to write (V62, tsk96) — the decomposable aggregate of
/// the facts sharing a `(capture, promoted dims)` grain. Built by
/// `metric_engine::Cell`, which owns the arithmetic; this is just the wire shape.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCubeRow {
    /// The producer whose live facts this bucket holds — not necessarily the
    /// capture's own producer.
    pub producer: String,
    pub dims_key: String,
    pub fact_count: i64,
    pub value_sum: f64,
    pub value_min: Option<f64>,
    pub value_max: Option<f64>,
    pub numerator: f64,
    pub denominator: f64,
}

/// One capture's live-partition mutation within a build batch (tsk113) — the
/// fold's step (`evict restated, insert own`), precomputed by the builder.
#[derive(Debug, Clone)]
pub struct BatchApply {
    pub branch: Option<String>,
    pub producer: String,
    pub restated: Vec<String>,
    pub inserted: Vec<(String, i64)>,
}

/// One capture's cube rows + watermark advance within a build batch (tsk113).
#[derive(Debug, Clone)]
pub struct BatchRows {
    pub branch: Option<String>,
    pub capture_id: i64,
    pub captured_at: Timestamp,
    pub rows: Vec<NewCubeRow>,
}

/// A cube bucket joined to its capture's spine — everything a `SeriesPoint` needs
/// without touching a single fact row. The capture attributes come from the JOIN
/// rather than being denormalized into `metric_cube`: a capture IS one scan/run,
/// so this is the ordinary star-schema shape (aggregate fact + shared dimension),
/// and it's what keeps branch/thread/stream/snapshot reachable from the cube.
#[derive(Debug, Clone, PartialEq)]
pub struct CubeReadRow {
    /// The producer whose live facts this bucket holds — the key the read's
    /// "producers that ever emitted a matching fact" derivation needs.
    pub producer: String,
    pub dims_key: String,
    pub fact_count: i64,
    pub value_sum: f64,
    pub value_min: Option<f64>,
    pub value_max: Option<f64>,
    pub numerator: f64,
    pub denominator: f64,
    pub capture_id: i64,
    pub captured_at: Timestamp,
    pub stream_id: i64,
    /// The CAPTURE's producer — what the capture list is filtered on. Distinct
    /// from `producer` above: a capture by `nextest` still carries `bun-test`'s
    /// live facts in the state it folds to.
    pub capture_producer: String,
    pub branch: Option<String>,
    pub provenance: String,
    pub source: String,
    pub closest_git_version: Option<String>,
}

/// The joined read view of a fact: its own measurement columns PLUS the spine it
/// inherits from its capture (`captured_at`, `branch`, version, effort, trust).
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
    /// The latest contributing CAPTURE (the capture is the run, T-E1), for
    /// the findings drill-in. Field name kept for wire compatibility.
    pub latest_run_id: Option<i64>,
}

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
    /// The capture's producer (gauge key / ingest kind) — identifies which scan
    /// emitted the fact, so reads can zero-fill a producer's EMPTY captures and
    /// scope "latest scan" currency per (stream, producer) (tsk44).
    pub producer: String,
}

const FACT_ROW_COLS: &str = "f.id, f.capture_id, f.measure_id, f.value, f.numerator, \
     f.denominator, f.subject_kind, f.subject_ref, f.path, f.line, f.severity, f.rule, \
     f.detail, f.dims_json, c.captured_at, c.branch, c.closest_git_version, \
     c.git_version_exact, c.basis_ref, c.snapshot_id, c.stream_id, c.thread_id, \
     c.effort_id, c.provenance, c.source, c.producer";

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
        producer: row.get(25)?,
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
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                // A `capture_scope` change swaps the cube's BUILD RULE (state
                // fold vs per-capture GROUP BY), so rows built under the old
                // rule must not survive to be served — invalidate that
                // measure's cube in the same transaction (tsk103 review).
                // Change-detected, never unconditional: `seed_catalog`
                // re-upserts every measure at boot (the tsk100 lesson).
                let prior_scope: Option<String> = tx
                    .query_row(
                        "SELECT capture_scope FROM measure WHERE key = ?1",
                        params![m.key],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(map_sql_err)?;
                let now = ts_to_string(Timestamp::now());
                // `component_role` is omitted — it defaults to 'none' and is
                // never read (dead V43 column, tsk15).
                tx.execute(
                    "INSERT INTO measure
                       (key, title, unit, subject_kind, temporal_semantics, capture_scope,
                        scope, description, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                     ON CONFLICT(key) DO UPDATE SET
                        title=excluded.title, unit=excluded.unit,
                        subject_kind=excluded.subject_kind,
                        temporal_semantics=excluded.temporal_semantics,
                        capture_scope=excluded.capture_scope,
                        scope=excluded.scope,
                        description=excluded.description, updated_at=excluded.updated_at",
                    params![
                        m.key,
                        m.title,
                        m.unit,
                        m.subject_kind,
                        m.temporal_semantics,
                        m.capture_scope,
                        m.scope,
                        m.description,
                        now,
                    ],
                )
                .map_err(map_sql_err)?;
                let id: i64 = tx
                    .query_row(
                        "SELECT id FROM measure WHERE key = ?1",
                        params![m.key],
                        |r| r.get(0),
                    )
                    .map_err(map_sql_err)?;
                if prior_scope.is_some_and(|p| p != m.capture_scope) {
                    for sql in [
                        "DELETE FROM metric_cube WHERE measure_id = ?1",
                        "DELETE FROM metric_live_fact WHERE measure_id = ?1",
                        "DELETE FROM metric_cube_state WHERE measure_id = ?1",
                    ] {
                        tx.execute(sql, params![id]).map_err(map_sql_err)?;
                    }
                    // Fence any build in flight (tsk103).
                    tx.execute("UPDATE metric_cube_epoch SET epoch = epoch + 1", [])
                        .map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(id)
            })
            .await
    }

    pub async fn get_measure(&self, key: &str) -> Result<Option<Measure>, DomainError> {
        let key = key.to_string();
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {MEASURE_COLS} FROM measure WHERE key = ?1");
                conn.prepare_cached(&sql)?
                    .query_row(params![key], row_to_measure)
                    .optional()
            })
            .await
    }

    pub async fn list_measures(&self) -> Result<Vec<Measure>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {MEASURE_COLS} FROM measure ORDER BY key");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt.query_map([], row_to_measure)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Insert or update (by `key`) a dimension in the conformed catalog.
    ///
    /// A change to `promoted` changes the cube's GRAIN (`dims_key` buckets by
    /// every promoted dim), so it invalidates the WHOLE cube in the same
    /// transaction — otherwise a pre-promotion bucket lacks the new key and a
    /// newly-eligible `dim_eq`/`group_by` read serves explicit 0s over real
    /// history (tsk103 review; V64 states the rule its migration honors by
    /// hand). Change-detected — `seed_catalog` re-upserts every dim at boot,
    /// and an unconditional wipe would re-fold the cube every start (tsk100's
    /// lesson). A brand-new dim arriving already-promoted also clears: facts
    /// may have carried the key in `dims_json` before the catalog knew it.
    pub async fn upsert_dimension(&self, d: NewDimension) -> Result<(), DomainError> {
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let prior: Option<bool> = tx
                    .query_row(
                        "SELECT promoted FROM dimension WHERE key = ?1",
                        params![d.key],
                        |r| r.get::<_, i64>(0).map(|v| v != 0),
                    )
                    .optional()
                    .map_err(map_sql_err)?;
                tx.execute(
                    "INSERT INTO dimension (key, label, value_type, subject_kind, vocabulary_json, scope, promoted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(key) DO UPDATE SET
                        label=excluded.label, value_type=excluded.value_type,
                        subject_kind=excluded.subject_kind,
                        vocabulary_json=excluded.vocabulary_json, scope=excluded.scope,
                        promoted=excluded.promoted",
                    params![d.key, d.label, d.value_type, d.subject_kind, d.vocabulary_json, d.scope, d.promoted],
                )
                .map_err(map_sql_err)?;
                let grain_changed = match prior {
                    Some(was) => was != d.promoted,
                    None => d.promoted,
                };
                if grain_changed {
                    for sql in [
                        "DELETE FROM metric_cube",
                        "DELETE FROM metric_live_fact",
                        "DELETE FROM metric_cube_state",
                        // Fence any build in flight (tsk103).
                        "UPDATE metric_cube_epoch SET epoch = epoch + 1",
                    ] {
                        tx.execute(sql, []).map_err(map_sql_err)?;
                    }
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    pub async fn list_dimensions(&self) -> Result<Vec<Dimension>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!("SELECT {DIM_COLS} FROM dimension ORDER BY key");
                let mut stmt = conn.prepare_cached(&sql)?;
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

    /// Remove a spec by key (idempotent — a missing key is a no-op). The prune
    /// primitive `seed_catalog` uses to reconcile the `metric_spec` table down to
    /// exactly the enabled set (a disabled metric's row is deleted so all
    /// spec-driven reads go empty).
    pub async fn delete_spec(&self, key: &str) -> Result<(), DomainError> {
        let key = key.to_string();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM metric_spec WHERE key = ?1", params![key])?;
                Ok(())
            })
            .await
    }

    /// Whether any spec currently sources this measure. Because `seed_catalog`
    /// prunes disabled specs, the `metric_spec` table equals the *enabled* set —
    /// so this is the producer collection gate: no active metric consumes the
    /// measure ⇒ the producer skips writing its facts (stop-collecting).
    pub async fn measure_has_active_spec(&self, measure_key: &str) -> Result<bool, DomainError> {
        let measure_key = measure_key.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM metric_spec WHERE source_measure = ?1)",
                    params![measure_key],
                    |r| r.get::<_, bool>(0),
                )
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
                // Idempotent ingestion (tsk14): if this capture carries a
                // content identity that's already been recorded, skip the whole
                // write (no duplicate capture, no double-counted facts) and
                // return the existing id. The partial unique index is the true
                // guard; this SELECT is the fast, race-free path on the single
                // serialized write connection.
                if let Some(key) = capture.idempotency_key.as_deref() {
                    let existing: Option<i64> = tx
                        .query_row(
                            "SELECT id FROM metric_capture WHERE idempotency_key = ?1",
                            params![key],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(map_sql_err)?;
                    if let Some(id) = existing {
                        return Ok(id);
                    }
                }
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

    /// The captures produced BY an effort (`effort_id` stamped on the capture) —
    /// the attribution-by-claim spine for the effort roll-up (epic tsk12, T-D).
    /// Oldest first.
    pub async fn captures_for_effort(
        &self,
        effort_id: i64,
    ) -> Result<Vec<MetricCapture>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {CAPTURE_COLS} FROM metric_capture
                      WHERE effort_id = ?1
                      ORDER BY captured_at ASC, id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![effort_id], row_to_capture)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Captures on a thread in a time window, filtered by `trigger` — the
    /// unified OBSERVE for run attribution now that the capture IS the run
    /// (T-E1, tsk48). All agent-work runs (tests/coverage/analysis) stamp
    /// `trigger = "on-report"` regardless of their (per-analyzer, varying)
    /// producer, so one filter covers all three. Oldest-first.
    pub async fn captures_in_window_by_trigger(
        &self,
        thread_id: i64,
        trigger: &str,
        start: Timestamp,
        end: Option<Timestamp>,
    ) -> Result<Vec<MetricCapture>, DomainError> {
        let trigger = trigger.to_string();
        let start = ts_to_string(start);
        let end = end.map(ts_to_string);
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {CAPTURE_COLS} FROM metric_capture
                      WHERE thread_id = ?1 AND trigger = ?2
                        AND captured_at >= ?3
                        AND (?4 IS NULL OR captured_at <= ?4)
                      ORDER BY captured_at ASC, id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows =
                    stmt.query_map(params![thread_id, trigger, start, end], row_to_capture)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Every capture recorded by the given producers (gauge keys / ingest
    /// kinds), oldest first — INCLUDING empty captures (a scan that found zero
    /// offenders writes a capture with no facts). The engine zero-fills a
    /// series from these so a count metric can drop back to zero (tsk44).
    pub async fn captures_for_producers(
        &self,
        producers: Vec<String>,
    ) -> Result<Vec<MetricCapture>, DomainError> {
        if producers.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |conn| {
                let placeholders = (1..=producers.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                // `status = 'done'` keeps the doc rule "non-done captures are
                // invisible to every fold" true for the in-memory fold and the
                // cube build, not just the SQL folds (tsk103 review): a failed
                // capture is a recorded event, never a data point — folded in,
                // it emits a phantom repeat of prior state, and a complete-
                // scope count/sum would zero-splice it.
                let sql = format!(
                    "SELECT {CAPTURE_COLS} FROM metric_capture
                      WHERE producer IN ({placeholders}) AND status = 'done'
                      ORDER BY captured_at ASC, id ASC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows =
                    stmt.query_map(rusqlite::params_from_iter(producers.iter()), row_to_capture)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
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

    /// [`Self::facts_for_measure`] bounded to one stream, SQL-side (tsk75).
    /// The effort delta reads are per-worktree by definition — loading every
    /// stream's history just to drop it in Rust made each panel refetch pay
    /// for the whole table.
    pub async fn facts_for_measure_in_stream(
        &self,
        measure_id: i64,
        stream_id: i64,
    ) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.measure_id = ?1 AND c.stream_id = ?2
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![measure_id, stream_id], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    // --- the aggregate cube (V62, tsk96) --------------------------------
    //
    // The cube is an ACCELERATOR and is DISPOSABLE: every row here is derivable
    // from `fact`, and dropping them all costs only speed. Nothing may read data
    // from the cube that the facts don't have. See `.context/metrics.md`.

    /// Every producer that has ever emitted a fact for this measure — the cube
    /// builder's capture-list seed.
    ///
    /// The builder is spec-INDEPENDENT (one cube serves every spec over the
    /// measure), so it folds every producer's captures. The read needs the
    /// narrower "producers matching THIS spec's filter" and derives that from the
    /// cube's own buckets rather than from the facts — deriving it from the facts
    /// is the 374k-row decode the cube exists to remove.
    /// Driven from CAPTURES (thousands) probing `idx_fact_measure_capture`, not
    /// from a DISTINCT over the measure's facts (hundreds of thousands). Same
    /// answer, ~4× cheaper on real data (8ms vs 32ms for `oxplow.test_case`) —
    /// and the builder runs this on every recording, so the constant matters.
    pub async fn producers_for_measure(&self, measure_id: i64) -> Result<Vec<String>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT DISTINCT c.producer FROM metric_capture c
                      WHERE EXISTS (SELECT 1 FROM fact f
                                     WHERE f.capture_id = c.id AND f.measure_id = ?1)",
                )?;
                let rows = stmt.query_map(params![measure_id], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// How far the cube is built for `(measure, stream)` — the newest capture
    /// folded in, as `(captured_at, id)` so it compares on the same key the fold
    /// orders by. `None` ⇒ nothing cubed yet.
    ///
    /// This is what disambiguates "no cube rows for capture N": state legitimately
    /// empty at N (a real value-0 point) vs N not cubed yet (fall back to facts).
    ///
    /// `metric_cube_state` rows are per BRANCH (V63); the stream's watermark is
    /// the MAX across them, which equals "the last capture folded" because the
    /// build processes a stream's captures in global `(captured_at, id)` order —
    /// every row it advances is the newest so far.
    pub async fn cube_watermark(
        &self,
        measure_id: i64,
        stream_id: i64,
    ) -> Result<Option<(Timestamp, i64)>, DomainError> {
        self.db
            .call(move |conn| {
                conn.prepare_cached(
                    "SELECT last_captured_at, last_capture_id FROM metric_cube_state
                      WHERE measure_id = ?1 AND stream_id = ?2
                      ORDER BY last_captured_at DESC, last_capture_id DESC
                      LIMIT 1",
                )?
                .query_row(params![measure_id, stream_id], |r| {
                    let at: String = r.get(0)?;
                    Ok((at, r.get::<_, i64>(1)?))
                })
                .optional()
            })
            .await?
            .map(|(at, id)| Ok((string_to_ts(&at).map_err(ts_conv_err)?, id)))
            .transpose()
            .map_err(map_sql_err)
    }

    /// The cube's global invalidation EPOCH — bumped by every wipe (prune with
    /// drops, a dim's promoted flip, a measure's scope change). The builder
    /// reads it before folding and `write_cube_rows` refuses to commit when it
    /// moved, so a wipe landing MID build can't be followed by a stale write
    /// that re-plants a watermark over rowless captures (tsk103 review).
    pub async fn cube_epoch(&self) -> Result<i64, DomainError> {
        self.db
            .call(move |conn| {
                conn.prepare_cached("SELECT epoch FROM metric_cube_epoch WHERE id = 1")?
                    .query_row([], |r| r.get(0))
            })
            .await
    }

    /// Whether `(measure, stream, branch)` has a live-state partition yet — the
    /// existence of its `metric_cube_state` row. `false` means the branch's first
    /// capture hasn't been folded and the build must SEED the partition by
    /// replaying the history visible to it. Existence, not row count: a seeded
    /// partition may legitimately hold zero live facts.
    pub async fn cube_branch_seeded(
        &self,
        measure_id: i64,
        stream_id: i64,
        branch: Option<String>,
    ) -> Result<bool, DomainError> {
        // `''` = "no branch" throughout the cube tables (a WITHOUT ROWID PK
        // can't hold NULL). Known, accepted collision: a capture recording
        // `Some("")` would share the partition `None` gets — but git forbids
        // empty branch names and no producer fabricates one, and the fact
        // fold keys on `Option`, where they'd differ (tsk109 audit note).
        let branch = branch.unwrap_or_default();
        self.db
            .call(move |conn| {
                conn.prepare_cached(
                    "SELECT 1 FROM metric_cube_state
                      WHERE measure_id = ?1 AND stream_id = ?2 AND branch = ?3",
                )?
                .query_row(params![measure_id, stream_id, branch], |_| Ok(()))
                .optional()
                .map(|r| r.is_some())
            })
            .await
    }

    /// The facts currently LIVE for `(measure, stream, branch)` — one branch
    /// partition of the fold's state, read back as whole facts so the caller
    /// buckets them with the same `dim_value` the read path uses (never a second
    /// dim-extraction implementation in SQL).
    pub async fn live_facts(
        &self,
        measure_id: i64,
        stream_id: i64,
        branch: Option<String>,
    ) -> Result<Vec<FactRow>, DomainError> {
        let branch = branch.unwrap_or_default();
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM metric_live_fact lf
                       JOIN fact f ON f.id = lf.fact_id
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE lf.measure_id = ?1 AND lf.stream_id = ?2 AND lf.branch = ?3
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows =
                    stmt.query_map(params![measure_id, stream_id, branch], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Establish a branch's live partition in ONE transaction: drop whatever the
    /// partition holds and insert `(producer, subject_key, fact_id)` rows — the
    /// final state of the builder's in-memory replay of the history visible to
    /// this branch. Atomic so a torn seed leaves no half-partition: the branch's
    /// `metric_cube_state` row (the seeded marker) only lands later, with its
    /// first `write_cube_rows`, so a crash between the two re-seeds from scratch.
    pub async fn seed_live_state(
        &self,
        measure_id: i64,
        stream_id: i64,
        branch: Option<String>,
        facts: Vec<(String, String, i64)>,
    ) -> Result<(), DomainError> {
        let branch = branch.unwrap_or_default();
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                tx.execute(
                    "DELETE FROM metric_live_fact
                      WHERE measure_id = ?1 AND stream_id = ?2 AND branch = ?3",
                    params![measure_id, stream_id, branch],
                )
                .map_err(map_sql_err)?;
                {
                    let mut insert = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO metric_live_fact
                               (measure_id, stream_id, branch, producer, subject_key, fact_id)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        )
                        .map_err(map_sql_err)?;
                    for (producer, key, fact_id) in &facts {
                        insert
                            .execute(params![
                                measure_id, stream_id, branch, producer, key, fact_id
                            ])
                            .map_err(map_sql_err)?;
                    }
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Replace a capture's cube rows and advance its BRANCH's watermark,
    /// atomically. The delete makes a re-run of the same capture idempotent. The
    /// upsert's insert arm is also what creates the branch's `metric_cube_state`
    /// row — the "seeded" marker `cube_branch_seeded` reads.
    ///
    /// Returns `false` — writing NOTHING — when the cube epoch moved past
    /// `expected_epoch`: an invalidation landed after the builder planned this
    /// pass, so its in-memory progress describes wiped state. The stale pass
    /// must abandon; the next build folds from the post-wipe watermark.
    // Eight primitives, all storage-layer plumbing with distinct types-of-
    // meaning; a param struct would add ceremony at every call site for no
    // reader gain (same call CollectionService::new makes).
    #[allow(clippy::too_many_arguments)]
    pub async fn write_cube_rows(
        &self,
        measure_id: i64,
        stream_id: i64,
        branch: Option<String>,
        capture_id: i64,
        captured_at: Timestamp,
        rows: Vec<NewCubeRow>,
        expected_epoch: i64,
    ) -> Result<bool, DomainError> {
        let branch = branch.unwrap_or_default();
        let captured_at = ts_to_string(captured_at);
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let epoch: i64 = tx
                    .prepare_cached("SELECT epoch FROM metric_cube_epoch WHERE id = 1")
                    .map_err(map_sql_err)?
                    .query_row([], |r| r.get(0))
                    .map_err(map_sql_err)?;
                if epoch != expected_epoch {
                    return Ok(false);
                }
                tx.prepare_cached(
                    "DELETE FROM metric_cube WHERE measure_id = ?1 AND capture_id = ?2",
                )
                .map_err(map_sql_err)?
                .execute(params![measure_id, capture_id])
                .map_err(map_sql_err)?;
                {
                    let mut insert = tx
                        .prepare_cached(
                            "INSERT INTO metric_cube
                               (measure_id, capture_id, producer, dims_key, fact_count,
                                value_sum, value_min, value_max, numerator, denominator)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        )
                        .map_err(map_sql_err)?;
                    for r in &rows {
                        insert
                            .execute(params![
                                measure_id,
                                capture_id,
                                r.producer,
                                r.dims_key,
                                r.fact_count,
                                r.value_sum,
                                r.value_min,
                                r.value_max,
                                r.numerator,
                                r.denominator
                            ])
                            .map_err(map_sql_err)?;
                    }
                }
                tx.prepare_cached(
                    "INSERT INTO metric_cube_state
                       (measure_id, stream_id, branch, last_capture_id, last_captured_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(measure_id, stream_id, branch) DO UPDATE SET
                       last_capture_id = excluded.last_capture_id,
                       last_captured_at = excluded.last_captured_at",
                )
                .map_err(map_sql_err)?
                .execute(params![
                    measure_id,
                    stream_id,
                    branch,
                    capture_id,
                    captured_at
                ])
                .map_err(map_sql_err)?;
                tx.commit().map_err(map_sql_err)?;
                Ok(true)
            })
            .await
    }

    /// Every cube bucket for a measure, oldest capture first, joined to its
    /// capture's spine — the read's replacement for decoding the raw facts.
    /// `stream` bounds it to one worktree; `None` reads every stream (each row
    /// still carries its own, so an unscoped read is a UNION, never a merge).
    pub async fn cube_rows_for_measure(
        &self,
        measure_id: i64,
        stream: Option<i64>,
    ) -> Result<Vec<CubeReadRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = "SELECT mc.producer, mc.dims_key, mc.fact_count, mc.value_sum,
                                  mc.value_min, mc.value_max, mc.numerator, mc.denominator,
                                  c.id, c.captured_at, c.stream_id, c.producer, c.branch,
                                  c.provenance, c.source, c.closest_git_version
                             FROM metric_cube mc
                             JOIN metric_capture c ON c.id = mc.capture_id
                            WHERE mc.measure_id = ?1
                              AND (?2 IS NULL OR c.stream_id = ?2)
                            ORDER BY c.captured_at ASC, c.id ASC, mc.dims_key ASC";
                let mut stmt = conn.prepare_cached(sql)?;
                let rows = stmt.query_map(params![measure_id, stream], |r| {
                    let captured_at: String = r.get(9)?;
                    Ok(CubeReadRow {
                        producer: r.get(0)?,
                        dims_key: r.get(1)?,
                        fact_count: r.get(2)?,
                        value_sum: r.get(3)?,
                        value_min: r.get(4)?,
                        value_max: r.get(5)?,
                        numerator: r.get(6)?,
                        denominator: r.get(7)?,
                        capture_id: r.get(8)?,
                        captured_at: string_to_ts(&captured_at).map_err(ts_conv_err)?,
                        stream_id: r.get(10)?,
                        capture_producer: r.get(11)?,
                        branch: r.get(12)?,
                        provenance: r.get(13)?,
                        source: r.get(14)?,
                        closest_git_version: r.get(15)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The PATH-LESS, SUBJECT-LESS facts of a measure — agent-asserted repo
    /// scalars (`record_metric` with no subject). The per-path read supplements
    /// its tree fold with these (they have no path, so nothing supersedes them
    /// per-path); it used to load the measure's entire history to find the
    /// usually-zero of them (tsk75).
    pub async fn pathless_scalar_facts(
        &self,
        measure_id: i64,
        stream_id: Option<i64>,
    ) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.measure_id = ?1
                        AND f.path IS NULL AND f.subject_ref IS NULL
                        AND (?2 IS NULL OR c.stream_id = ?2)
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![measure_id, stream_id], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Delete PROJECT-scope metric specs whose key is not in `keep` (tsk61).
    /// A metric removed from `.oxplow/project.yaml` entirely (not merely
    /// `enabled: false`) used to leave a zombie spec row that rendered as a
    /// forever-blank gauge in the catalog. Built-in/global rows are never
    /// touched — the declared config is only the truth for its own scope.
    pub async fn delete_project_specs_not_in(&self, keep: Vec<String>) -> Result<u64, DomainError> {
        self.db
            .call(move |conn| {
                // `NOT IN ()` isn't valid SQL — an empty keep-set means "no
                // project metrics are declared", i.e. delete them all.
                if keep.is_empty() {
                    let n = conn.execute("DELETE FROM metric_spec WHERE scope = 'project'", [])?;
                    return Ok(n as u64);
                }
                let placeholders = std::iter::repeat("?")
                    .take(keep.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "DELETE FROM metric_spec
                      WHERE scope = 'project' AND key NOT IN ({placeholders})"
                );
                let n = conn.execute(&sql, rusqlite::params_from_iter(keep.iter()))?;
                Ok(n as u64)
            })
            .await
    }

    /// Delete PROJECT-scope measures whose key is not in `keep` (tsk61) — the
    /// measure-side of the same reconciliation. Facts CASCADE via
    /// `fact.measure_id`: a measure the user removed from config is retired,
    /// history included (the same declared-config-is-truth stance as specs).
    pub async fn delete_project_measures_not_in(
        &self,
        keep: Vec<String>,
    ) -> Result<u64, DomainError> {
        self.db
            .call(move |conn| {
                if keep.is_empty() {
                    let n = conn.execute("DELETE FROM measure WHERE scope = 'project'", [])?;
                    return Ok(n as u64);
                }
                let placeholders = std::iter::repeat("?")
                    .take(keep.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "DELETE FROM measure
                      WHERE scope = 'project' AND key NOT IN ({placeholders})"
                );
                let n = conn.execute(&sql, rusqlite::params_from_iter(keep.iter()))?;
                Ok(n as u64)
            })
            .await
    }

    /// Distinct producers of `done` captures recorded under `source` (tsk62).
    /// Seeds the zero-fill for measures whose producers are only discoverable
    /// from facts: an analyzer that has been CLEAN since day one has zero
    /// `oxplow.lint_hit` facts, so fact-derived producer discovery finds
    /// nothing and its "ran, found nothing" captures could never zero-fill —
    /// the metric read blank forever instead of 0.
    pub async fn producers_for_capture_source(
        &self,
        source: &str,
    ) -> Result<Vec<String>, DomainError> {
        let source = source.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT producer FROM metric_capture
                      WHERE source = ?1 AND status = 'done'",
                )?;
                let rows = stmt.query_map(params![source], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// One representative fact per distinct `(producer, rule, severity,
    /// dims_json)` slice of a measure (tsk75). The zero-splice fallback in the
    /// effort delta reads only needs to learn WHICH producers emit a metric's
    /// slice — it loaded the measure's entire history (191k rows) to extract a
    /// handful of distinct producer names. Slice combos are bounded (rules ×
    /// severities × dim payloads), never fact-count-shaped.
    pub async fn representative_facts_by_slice(
        &self,
        measure_id: i64,
    ) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.id IN (
                        SELECT MIN(f2.id) FROM fact f2
                          JOIN metric_capture c2 ON c2.id = f2.capture_id
                         WHERE f2.measure_id = ?1
                         GROUP BY c2.producer, f2.rule, f2.severity, f2.dims_json
                      )
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
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&measure_id];
                for id in &capture_ids {
                    binds.push(id);
                }
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The CURRENT facts of a `capture_scope = 'per-path'` measure (V54, tsk41):
    /// the incremental-tree fold.
    ///
    /// A tree gauge's capture restates only **the paths in its snapshot** (a
    /// per-commit delta), so "the last capture" is NOT the repo — it's the last
    /// few files. The repo state is instead: for each `(producer, path)`, the facts
    /// from the **latest capture of that producer whose snapshot contained that
    /// path**. Older captures' facts for that path are superseded.
    ///
    /// The scanned set is taken from the capture's snapshot's `file_snapshot` rows,
    /// NOT from the facts it emitted — which is what makes the whole thing work
    /// without any write-side convention:
    /// - a file whose count drops to **0** emits no fact, but its path is in the new
    ///   snapshot, so the new capture supersedes the stale value (contributes 0);
    /// - a **deleted** file's latest row is a `storage='deleted'` tombstone → dropped
    ///   (the same rule `SqliteSnapshotStore::tree_at` applies);
    /// - **symbol**-grained facts and **many-facts-per-path** (TODO markers) are
    ///   superseded *wholesale per file*, so a removed function/marker disappears.
    ///
    /// Partitioning by `producer` matters: the 10 idiom gauges all share
    /// `oxplow.ast_hit` (sliced by `rule`), so without it a later gauge's capture
    /// would supersede an earlier gauge's facts for the same path.
    ///
    /// Oldest-first, like the other fact reads.
    pub async fn latest_tree_facts(
        &self,
        measure_id: i64,
        stream_id: Option<i64>,
    ) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                // Partition by stream too: a stream is a worktree, and two worktrees
                // are two independent trees — one's scan must never supersede the
                // other's facts for the same path.
                let sql = format!(
                    "WITH rel AS (
                       -- Producers whose captures can possibly matter to THIS measure.
                       -- The fold partitions by producer and a fact only survives via
                       -- its OWN capture, so captures of unrelated producers can never
                       -- change the result — enumerating them just made every read pay
                       -- for every gauge's history (0.7s/read; ~20 reads blew the 5s
                       -- UserPromptSubmit hook budget).
                       SELECT DISTINCT c2.producer AS producer
                         FROM fact f2 JOIN metric_capture c2 ON c2.id = f2.capture_id
                        WHERE f2.measure_id = ?1
                     ),
                     anchor_tree AS (
                       -- The reconstructed tree per DISTINCT full-capture anchor
                       -- (`tree_at` semantics: latest row per path <= the anchor,
                       -- tombstones included). Reconstructed once per anchor, not per
                       -- capture — a boot baseline anchors ~30 gauges to one snapshot.
                       SELECT stream_id, anchor, path, storage FROM (
                         SELECT a.stream_id AS stream_id, a.snapshot_id AS anchor,
                                fs.path AS path, fs.storage AS storage,
                                ROW_NUMBER() OVER (
                                  PARTITION BY a.stream_id, a.snapshot_id, fs.path
                                  ORDER BY fs.snapshot_id DESC, fs.id DESC
                                ) AS rn
                           FROM (SELECT DISTINCT stream_id, snapshot_id
                                   FROM metric_capture
                                  WHERE scan_kind = 'full' AND status = 'done'
                                    AND snapshot_id IS NOT NULL
                                    AND producer IN (SELECT producer FROM rel)) a
                           JOIN file_snapshot fs
                             ON fs.stream_id = a.stream_id
                            AND fs.snapshot_id IS NOT NULL
                            AND fs.snapshot_id <= a.snapshot_id
                       ) WHERE rn = 1
                     ),
                     restated AS (
                       -- A DELTA capture (the incremental rescan) restates every path in
                       -- its own snapshot — including deletion tombstones, which is how a
                       -- removed file drops out.
                       SELECT c.id AS capture_id, c.stream_id AS stream_id,
                              c.producer AS producer, c.captured_at AS captured_at,
                              fs.path AS path, fs.storage AS storage
                         FROM metric_capture c
                         JOIN file_snapshot fs
                           ON fs.snapshot_id = c.snapshot_id
                          AND fs.stream_id = c.stream_id
                        WHERE c.snapshot_id IS NOT NULL AND c.status = 'done'
                          AND c.scan_kind = 'delta'
                          AND c.producer IN (SELECT producer FROM rel)
                       UNION
                       -- A FULL capture (a baseline, tsk71) restates the RECONSTRUCTED
                       -- tree as-of its snapshot — which lets a baseline anchor to an
                       -- ordinary delta snapshot instead of a fabricated full-tree one.
                       -- Only the LATEST full capture per (stream, producer): an older
                       -- full capture covers a subset of a newer one's paths at an older
                       -- captured_at, so it can never win the rank — skipping it keeps
                       -- reads flat as forced rebuilds accumulate.
                       SELECT c.id, c.stream_id, c.producer, c.captured_at,
                              t.path, t.storage
                         FROM metric_capture c
                         JOIN anchor_tree t
                           ON t.stream_id = c.stream_id AND t.anchor = c.snapshot_id
                        WHERE c.scan_kind = 'full' AND c.status = 'done'
                          AND c.producer IN (SELECT producer FROM rel)
                          AND NOT EXISTS (
                            SELECT 1 FROM metric_capture c3
                             WHERE c3.stream_id = c.stream_id
                               AND c3.producer = c.producer
                               AND c3.scan_kind = 'full' AND c3.status = 'done'
                               AND (c3.captured_at > c.captured_at
                                    OR (c3.captured_at = c.captured_at AND c3.id > c.id))
                          )
                       UNION
                       -- An ASSERTED capture (agent `record_metric`, synthetic writes)
                       -- restates exactly the paths it emitted facts for; its snapshot,
                       -- when present, is provenance only — never a scanned set.
                       SELECT c.id, c.stream_id, c.producer, c.captured_at,
                              f.path, 'oxplow'
                         FROM metric_capture c
                         JOIN fact f ON f.capture_id = c.id
                        WHERE c.scan_kind = 'asserted' AND f.path IS NOT NULL
                          AND c.status = 'done'
                          AND c.producer IN (SELECT producer FROM rel)
                     ),
                     ranked AS (
                       SELECT capture_id, path, storage,
                              ROW_NUMBER() OVER (
                                PARTITION BY stream_id, producer, path
                                ORDER BY captured_at DESC, capture_id DESC
                              ) AS rn
                         FROM restated
                        WHERE (?2 IS NULL OR stream_id = ?2)
                     )
                     SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                       JOIN ranked s ON s.capture_id = f.capture_id AND s.path = f.path
                      WHERE f.measure_id = ?1
                        AND s.rn = 1
                        AND s.storage <> 'deleted'
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![measure_id, stream_id], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Whether a producer already has a `done` capture for `snapshot_id` at exactly
    /// `version` (its current fingerprint). The idempotency guard for a whole-tree
    /// sweep (tsk50): a re-delivered snapshot event — or a direct baseline run *plus*
    /// the event loop reacting to the same snapshot — must not re-scan the tree.
    /// `version = None` always returns `false` (can't confirm the logic matches, so
    /// don't skip).
    pub async fn gauge_done_for_snapshot(
        &self,
        producer: &str,
        snapshot_id: i64,
        version: Option<&str>,
        scan_kind: &str,
    ) -> Result<bool, DomainError> {
        let Some(version) = version else {
            return Ok(false);
        };
        let producer = producer.to_string();
        let version = version.to_string();
        let scan_kind = scan_kind.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    // Kind-scoped: a delta capture for this snapshot must not
                    // satisfy a pending FULL baseline run over it (tsk71) — the
                    // two scans cover different sets.
                    "SELECT EXISTS(
                       SELECT 1 FROM metric_capture
                        WHERE producer = ?1 AND snapshot_id = ?2
                          AND status = 'done' AND producer_version = ?3
                          AND scan_kind = ?4
                     )",
                    params![producer, snapshot_id, version, scan_kind],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n != 0)
            })
            .await
    }

    /// Drop the tree captures a newer baseline has made dead weight (tsk75).
    ///
    /// By the tsk71 dominance argument, an **effort-less** `delta`/`full`
    /// capture strictly OLDER than its (stream, producer)'s latest done `full`
    /// capture can never win any per-path fold rank: the baseline restates
    /// every path it ever scanned (live rows AND tombstones), newer. Their
    /// facts are pure dead weight — the per-function code measures had
    /// accumulated ~178k such rows EACH (~69% of the fact table), and every
    /// full-history read paid for them. Facts cascade via
    /// `fact.capture_id ON DELETE CASCADE`.
    ///
    /// Deliberately narrow:
    /// - **effort-stamped captures survive** — they're attribution history
    ///   (`captures_for_effort` reads them for closed-effort panels);
    /// - captures carrying any fact on a **non-per-path measure** survive —
    ///   complete/per-subject folds read past captures;
    /// - producers with **no full capture** are untouched;
    /// - `asserted`/`failed` captures are untouched (assertions are history,
    ///   failure records are gauge-health evidence).
    ///
    /// Trade-off, accepted deliberately: a per-path measure's TREND loses its
    /// pre-baseline points (the current fold and every effort window at/after
    /// the baseline are unaffected). Runs after each successful full sweep.
    ///
    /// **Invalidates that stream's cube when it drops anything** (tsk100). Deleted
    /// captures' facts cascade, and `metric_live_fact` cascades with them — but
    /// `metric_cube` rows are frozen at build time and would keep counting a fact
    /// the facts no longer have. Usually they'd agree anyway (the baseline already
    /// evicted those paths), but not for a path the sweep never restated, so we
    /// invalidate rather than reason about which prunes are safe. The cube is
    /// disposable; the next build re-folds. A prune that drops NOTHING leaves it
    /// alone — `rebuild_metric_baseline` prunes on every boot, and wiping a healthy
    /// cube each start would turn tsk96's fix off for nothing.
    pub async fn prune_dominated_tree_captures(&self, stream_id: i64) -> Result<u64, DomainError> {
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let n = tx
                    .execute(
                        "DELETE FROM metric_capture
                      WHERE id IN (
                        SELECT c.id
                          FROM metric_capture c
                          JOIN (
                            SELECT stream_id, producer, captured_at, id
                              FROM (
                                SELECT stream_id, producer, captured_at, id,
                                       ROW_NUMBER() OVER (
                                         PARTITION BY stream_id, producer
                                         ORDER BY captured_at DESC, id DESC
                                       ) AS rn
                                  FROM metric_capture
                                 WHERE scan_kind = 'full' AND status = 'done'
                              ) WHERE rn = 1
                          ) lf ON lf.stream_id = c.stream_id AND lf.producer = c.producer
                         WHERE c.stream_id = ?1
                           AND c.effort_id IS NULL
                           AND c.status = 'done'
                           AND c.scan_kind IN ('delta', 'full')
                           AND (c.captured_at < lf.captured_at
                                OR (c.captured_at = lf.captured_at AND c.id < lf.id))
                           AND NOT EXISTS (
                             SELECT 1 FROM fact f
                               JOIN measure m ON m.id = f.measure_id
                              WHERE f.capture_id = c.id
                                AND m.capture_scope <> 'per-path'
                           )
                      )",
                        params![stream_id],
                    )
                    .map_err(map_sql_err)?;
                // Same transaction as the delete: the cube must never be observable
                // as "built" over history that no longer exists.
                if n > 0 {
                    for sql in [
                        "DELETE FROM metric_cube WHERE capture_id IN
                           (SELECT id FROM metric_capture WHERE stream_id = ?1)",
                        "DELETE FROM metric_live_fact WHERE stream_id = ?1",
                        "DELETE FROM metric_cube_state WHERE stream_id = ?1",
                    ] {
                        tx.execute(sql, params![stream_id]).map_err(map_sql_err)?;
                    }
                    // Fence any build already in flight (tsk103): its todo-list
                    // predates this wipe, so its next write must abandon.
                    tx.execute("UPDATE metric_cube_epoch SET epoch = epoch + 1", [])
                        .map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(n as u64)
            })
            .await
    }

    /// Apply one BUILD BATCH — a chunk of captures' folds — in a single
    /// transaction (tsk113). Per step, in capture order: evict+insert the
    /// live partition, replace the capture's cube rows, advance its branch's
    /// watermark. ONE epoch check guards the whole chunk; `false` means an
    /// invalidation landed after the builder planned it — nothing is written,
    /// the stale pass abandons.
    ///
    /// Batching is what the profile asked for (one tiny transaction per
    /// capture rewrote the same hot B-tree pages into the WAL ~10k times per
    /// backfill) and it STRENGTHENS the crash story: a torn chunk lands
    /// nothing, and re-running it replays whole captures idempotently.
    pub async fn apply_build_batch(
        &self,
        measure_id: i64,
        stream_id: i64,
        steps: Vec<(Option<BatchApply>, BatchRows)>,
        expected_epoch: i64,
    ) -> Result<bool, DomainError> {
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let epoch: i64 = tx
                    .prepare_cached("SELECT epoch FROM metric_cube_epoch WHERE id = 1")
                    .map_err(map_sql_err)?
                    .query_row([], |r| r.get(0))
                    .map_err(map_sql_err)?;
                if epoch != expected_epoch {
                    return Ok(false);
                }
                for (apply, rows) in &steps {
                    if let Some(a) = apply {
                        let branch = a.branch.clone().unwrap_or_default();
                        let mut evict = tx
                            .prepare_cached(
                                "DELETE FROM metric_live_fact
                                  WHERE measure_id = ?1 AND stream_id = ?2 AND branch = ?3
                                    AND producer = ?4 AND subject_key = ?5",
                            )
                            .map_err(map_sql_err)?;
                        for key in &a.restated {
                            evict
                                .execute(params![measure_id, stream_id, branch, a.producer, key])
                                .map_err(map_sql_err)?;
                        }
                        let mut insert = tx
                            .prepare_cached(
                                "INSERT OR IGNORE INTO metric_live_fact
                                   (measure_id, stream_id, branch, producer, subject_key, fact_id)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            )
                            .map_err(map_sql_err)?;
                        for (key, fact_id) in &a.inserted {
                            insert
                                .execute(params![
                                    measure_id, stream_id, branch, a.producer, key, fact_id
                                ])
                                .map_err(map_sql_err)?;
                        }
                    }
                    let branch = rows.branch.clone().unwrap_or_default();
                    let captured_at = ts_to_string(rows.captured_at);
                    tx.prepare_cached(
                        "DELETE FROM metric_cube WHERE measure_id = ?1 AND capture_id = ?2",
                    )
                    .map_err(map_sql_err)?
                    .execute(params![measure_id, rows.capture_id])
                    .map_err(map_sql_err)?;
                    {
                        let mut insert = tx
                            .prepare_cached(
                                "INSERT INTO metric_cube
                                   (measure_id, capture_id, producer, dims_key, fact_count,
                                    value_sum, value_min, value_max, numerator, denominator)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            )
                            .map_err(map_sql_err)?;
                        for r in &rows.rows {
                            insert
                                .execute(params![
                                    measure_id,
                                    rows.capture_id,
                                    r.producer,
                                    r.dims_key,
                                    r.fact_count,
                                    r.value_sum,
                                    r.value_min,
                                    r.value_max,
                                    r.numerator,
                                    r.denominator
                                ])
                                .map_err(map_sql_err)?;
                        }
                    }
                    tx.prepare_cached(
                        "INSERT INTO metric_cube_state
                           (measure_id, stream_id, branch, last_capture_id, last_captured_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(measure_id, stream_id, branch) DO UPDATE SET
                           last_capture_id = excluded.last_capture_id,
                           last_captured_at = excluded.last_captured_at",
                    )
                    .map_err(map_sql_err)?
                    .execute(params![
                        measure_id,
                        stream_id,
                        branch,
                        rows.capture_id,
                        captured_at
                    ])
                    .map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(true)
            })
            .await
    }

    /// Prune metric captures older than `cutoff` — the OPT-IN retention knob
    /// (`metricRetentionDays`, tsk93; the default 0 means this is never
    /// called). Deletes ONLY history no current value stands on; kept
    /// unconditionally:
    /// - **effort-stamped captures** — attribution history;
    /// - each `(stream, producer)`'s **newest capture** — the headline /
    ///   zero-fill anchor;
    /// - any capture owning a **latest-per-partition fact** — latest per
    ///   `(measure, stream, producer, subject_ref)`, per `(…, path)`, or the
    ///   latest repo-scalar per `(measure, stream, producer)` — a
    ///   conservative superset of "live in some fold" regardless of the
    ///   measure's scope. Deleting a live fact would move TODAY's number,
    ///   which retention must never do.
    ///
    /// Points older than the cutoff disappear from series and drill-down
    /// (that IS retention — the trade the knob buys into). Facts cascade via
    /// FK; the affected streams' cube is invalidated in the same transaction
    /// and the epoch fenced (the tsk100 rule: replay inputs changed).
    pub async fn prune_aged_captures(&self, cutoff: Timestamp) -> Result<u64, DomainError> {
        let cutoff = ts_to_string(cutoff);
        self.db
            .call_mut(move |conn| {
                let tx = conn.transaction().map_err(map_sql_err)?;
                let doomed_where = "captured_at < ?1
                       AND effort_id IS NULL
                       AND id NOT IN (
                         SELECT id FROM (
                           SELECT id, ROW_NUMBER() OVER (
                             PARTITION BY stream_id, producer
                             ORDER BY captured_at DESC, id DESC) rn
                           FROM metric_capture)
                         WHERE rn = 1)
                       AND id NOT IN (
                         SELECT capture_id FROM (
                           SELECT f.capture_id, ROW_NUMBER() OVER (
                             PARTITION BY f.measure_id, c.stream_id, c.producer, f.subject_ref
                             ORDER BY c.captured_at DESC, c.id DESC, f.id DESC) rn
                           FROM fact f JOIN metric_capture c ON c.id = f.capture_id
                           WHERE f.subject_ref IS NOT NULL AND c.status = 'done')
                         WHERE rn = 1)
                       AND id NOT IN (
                         SELECT capture_id FROM (
                           SELECT f.capture_id, ROW_NUMBER() OVER (
                             PARTITION BY f.measure_id, c.stream_id, c.producer, f.path
                             ORDER BY c.captured_at DESC, c.id DESC, f.id DESC) rn
                           FROM fact f JOIN metric_capture c ON c.id = f.capture_id
                           WHERE f.path IS NOT NULL AND c.status = 'done')
                         WHERE rn = 1)
                       AND id NOT IN (
                         SELECT capture_id FROM (
                           SELECT f.capture_id, ROW_NUMBER() OVER (
                             PARTITION BY f.measure_id, c.stream_id, c.producer
                             ORDER BY c.captured_at DESC, c.id DESC, f.id DESC) rn
                           FROM fact f JOIN metric_capture c ON c.id = f.capture_id
                           WHERE f.subject_ref IS NULL AND f.path IS NULL
                             AND c.status = 'done')
                         WHERE rn = 1)";
                let mut streams: Vec<i64> = {
                    let sql = format!(
                        "SELECT DISTINCT stream_id FROM metric_capture WHERE {doomed_where}"
                    );
                    let mut stmt = tx.prepare(&sql).map_err(map_sql_err)?;
                    let rows = stmt
                        .query_map(params![cutoff], |r| r.get::<_, i64>(0))
                        .map_err(map_sql_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(map_sql_err)?;
                    rows
                };
                streams.sort_unstable();
                let n = tx
                    .execute(
                        &format!("DELETE FROM metric_capture WHERE {doomed_where}"),
                        params![cutoff],
                    )
                    .map_err(map_sql_err)?;
                if n > 0 {
                    for stream_id in streams {
                        for sql in [
                            "DELETE FROM metric_cube WHERE capture_id IN
                               (SELECT id FROM metric_capture WHERE stream_id = ?1)",
                            "DELETE FROM metric_live_fact WHERE stream_id = ?1",
                            "DELETE FROM metric_cube_state WHERE stream_id = ?1",
                        ] {
                            tx.execute(sql, params![stream_id]).map_err(map_sql_err)?;
                        }
                    }
                    // Fence any build already in flight (tsk103).
                    tx.execute("UPDATE metric_cube_epoch SET epoch = epoch + 1", [])
                        .map_err(map_sql_err)?;
                }
                tx.commit().map_err(map_sql_err)?;
                Ok(n as u64)
            })
            .await
    }

    /// Whether a producer has EVER completed a `scan_kind = 'full'` baseline
    /// capture in this stream — at `version` when given, at any version when
    /// `None`. This is the "has this gauge been baselined" question (tsk71):
    /// a gauge with no full capture at its current fingerprint needs a
    /// full-tree run before its per-path metric is trustworthy.
    pub async fn has_full_capture(
        &self,
        producer: &str,
        stream_id: i64,
        version: Option<&str>,
    ) -> Result<bool, DomainError> {
        let producer = producer.to_string();
        let version = version.map(|v| v.to_string());
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM metric_capture
                        WHERE producer = ?1 AND stream_id = ?2
                          AND status = 'done' AND scan_kind = 'full'
                          AND (?3 IS NULL OR producer_version = ?3)
                     )",
                    params![producer, stream_id, version],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n != 0)
            })
            .await
    }

    /// The `status` of a producer's LATEST capture (`done` | `failed` | `running`),
    /// or `None` when it has never captured. Lets the runner tell "the gauge found
    /// nothing" apart from "the gauge blew up" (tsk47/tsk48).
    pub async fn latest_capture_status(
        &self,
        producer: &str,
        stream_id: i64,
    ) -> Result<Option<String>, DomainError> {
        let producer = producer.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT status FROM metric_capture
                      WHERE producer = ?1 AND stream_id = ?2
                      ORDER BY captured_at DESC, id DESC
                      LIMIT 1",
                    params![producer, stream_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
            })
            .await
    }

    /// The `producer_version` on a producer's LATEST capture (V56, tsk45), and
    /// whether it has ever captured at all.
    ///
    /// `Ok(None)` — the producer has no captures in this stream (never run).
    /// `Ok(Some(v))` — its latest capture recorded logic version `v` (`None` inside
    /// = an unversioned/pre-V56 capture). Compare against the gauge's current
    /// fingerprint: a mismatch means its facts were computed by different logic and
    /// a re-baseline is due.
    pub async fn latest_producer_version(
        &self,
        producer: &str,
        stream_id: i64,
    ) -> Result<Option<Option<String>>, DomainError> {
        let producer = producer.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT producer_version FROM metric_capture
                      WHERE producer = ?1 AND stream_id = ?2
                      ORDER BY captured_at DESC, id DESC
                      LIMIT 1",
                    params![producer, stream_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
            })
            .await
    }

    /// The CURRENT facts of a `capture_scope = 'per-subject'` measure (V55, tsk43).
    ///
    /// A capture restates only the **subjects it emitted facts for** — for
    /// `oxplow.test_case`, the test cases the run actually executed. So the value is
    /// the latest fact per `(producer, subject_ref)`: a PARTIAL test run updates just
    /// the tests it ran, and every other test keeps its last-known status. Read as
    /// `complete` ("the last capture restates every test") a partial run would make
    /// the metric report a 4-test repo.
    ///
    /// Unlike `per-path` there is no external scanned set to anchor on (a test run has
    /// no snapshot file rows), so the restated set IS the capture's own facts. The
    /// consequence is that a **deleted/renamed test lingers** — nothing can say "this
    /// subject no longer exists" the way a `storage='deleted'` file row can.
    ///
    /// Oldest-first, like the other fact reads.
    pub async fn latest_subject_facts(
        &self,
        measure_id: i64,
        stream_id: Option<i64>,
    ) -> Result<Vec<FactRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = format!(
                    "WITH ranked AS (
                       SELECT f.id AS fact_id,
                              ROW_NUMBER() OVER (
                                PARTITION BY c.stream_id, c.producer, f.subject_ref
                                ORDER BY c.captured_at DESC, c.id DESC, f.id DESC
                              ) AS rn
                         FROM fact f
                         JOIN metric_capture c ON c.id = f.capture_id
                        WHERE f.measure_id = ?1
                          AND f.subject_ref IS NOT NULL
                          AND c.status = 'done'
                          AND (?2 IS NULL OR c.stream_id = ?2)
                     )
                     SELECT {FACT_ROW_COLS} FROM fact f
                       JOIN metric_capture c ON c.id = f.capture_id
                       JOIN ranked r ON r.fact_id = f.id
                      WHERE r.rn = 1
                      ORDER BY c.captured_at ASC, f.id ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![measure_id, stream_id], row_to_fact_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The paths each capture SCANNED — i.e. the paths in its snapshot, including
    /// `deleted` tombstones (a deletion is a scan result: "this path is gone").
    /// Used by the engine's running fold to build a per-path trend line: at each
    /// capture, the paths it scanned are evicted from the running state and
    /// replaced by whatever facts it emitted for them. Empty when `capture_ids` is
    /// empty.
    pub async fn scanned_paths_for_captures(
        &self,
        capture_ids: Vec<i64>,
    ) -> Result<Vec<(i64, String)>, DomainError> {
        if capture_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |conn| {
                let placeholders = std::iter::repeat("?")
                    .take(capture_ids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                // Mirrors `latest_tree_facts`' three scan kinds: delta = the
                // snapshot's own paths; full = the reconstructed tree as-of the
                // snapshot; asserted = exactly the paths it emitted facts for.
                let sql = format!(
                    "SELECT c.id, fs.path
                       FROM metric_capture c
                       JOIN file_snapshot fs
                         ON fs.snapshot_id = c.snapshot_id
                        AND fs.stream_id = c.stream_id
                      WHERE c.snapshot_id IS NOT NULL AND c.status = 'done'
                        AND c.scan_kind = 'delta'
                        AND c.id IN ({placeholders})
                     UNION
                     SELECT capture_id, path FROM (
                       SELECT c.id AS capture_id, fs.path AS path,
                              ROW_NUMBER() OVER (
                                PARTITION BY c.id, fs.path
                                ORDER BY fs.snapshot_id DESC, fs.id DESC
                              ) AS tree_rn
                         FROM metric_capture c
                         JOIN file_snapshot fs
                           ON fs.stream_id = c.stream_id
                          AND fs.snapshot_id IS NOT NULL
                          AND fs.snapshot_id <= c.snapshot_id
                        WHERE c.snapshot_id IS NOT NULL AND c.status = 'done'
                          AND c.scan_kind = 'full'
                          AND c.id IN ({placeholders})
                     ) WHERE tree_rn = 1
                     UNION
                     SELECT c.id, f.path
                       FROM metric_capture c
                       JOIN fact f ON f.capture_id = c.id
                      WHERE c.scan_kind = 'asserted' AND f.path IS NOT NULL
                        AND c.status = 'done'
                        AND c.id IN ({placeholders})"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                // The id list appears in all THREE arms of the UNION, so bind it
                // three times.
                let binds = capture_ids
                    .iter()
                    .chain(capture_ids.iter())
                    .chain(capture_ids.iter());
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
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

    // --- per-path fold (V54, tsk41) helpers -------------------------------

    /// A snapshot on stream 1 carrying `files` as `(path, storage)` rows —
    /// `storage` is `"oxplow"` (present) or `"deleted"` (a tombstone). This is the
    /// gauge's SCANNED SET: the fold reads it to know which paths a capture
    /// restated.
    async fn snapshot_with(store: &SqliteFactStore, snap_id: i64, files: &[(&str, &str)]) {
        let files: Vec<(String, String)> = files
            .iter()
            .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
            .collect();
        let db = store.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let now = "2026-06-30T00:00:00Z";
                conn.execute(
                    "INSERT INTO snapshot (id, stream_id, created_at) VALUES (?1, 1, ?2)",
                    params![snap_id, now],
                )?;
                for (path, storage) in &files {
                    conn.execute(
                        "INSERT INTO file_snapshot
                           (stream_id, path, blob_hash, size_bytes, captured_at, snapshot_id, storage)
                         VALUES (1, ?1, 'h', 1, ?2, ?3, ?4)",
                        params![path, now, snap_id, storage],
                    )?;
                }
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
    }

    /// A gauge capture by `producer` over `snap_id`, emitting one fact per
    /// `(path, value)`. Mirrors what `record_gauge_facts` writes.
    async fn gauge_capture(
        store: &SqliteFactStore,
        producer: &str,
        snap_id: i64,
        captured_at: &str,
        measure_id: i64,
        facts: &[(&str, f64)],
    ) -> i64 {
        let mut capture = NewMetricCapture::done(1, producer, format!("metric:{producer}"));
        capture.snapshot_id = Some(snap_id);
        capture.captured_at = Some(at(captured_at));
        let rows: Vec<NewFact> = facts
            .iter()
            .map(|(path, value)| NewFact {
                subject_kind: Some("file".into()),
                subject_ref: Some((*path).to_string()),
                path: Some((*path).to_string()),
                ..NewFact::new(measure_id, *value)
            })
            .collect();
        store.record_facts(capture, rows).await.unwrap()
    }

    fn total(facts: &[FactRow]) -> f64 {
        facts.iter().map(|f| f.value).sum()
    }

    #[tokio::test]
    async fn per_path_fold_supersedes_a_rescanned_file_that_dropped_to_zero() {
        // THE core bug. Baseline: a.rs has 3, b.rs has 2 (total 5). Then a.rs is
        // edited to 0 — the gauge emits NO fact for it (the `if c > 0:` guard), but
        // a.rs IS in the new snapshot, so the new capture supersedes it → 2.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;
        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            5.0
        );

        // Only a.rs changed; it now has zero hits, so the gauge emits nothing.
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        gauge_capture(&store, "g", 2, "2026-06-30T11:00:00.000000Z", m, &[]).await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 2.0, "a.rs superseded to 0; b.rs unchanged");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].path.as_deref(), Some("b.rs"));
    }

    #[tokio::test]
    async fn per_path_fold_drops_a_deleted_file() {
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;

        // a.rs is deleted: its latest row is a tombstone. No gauge fact for it.
        snapshot_with(&store, 2, &[("a.rs", "deleted")]).await;
        gauge_capture(&store, "g", 2, "2026-06-30T11:00:00.000000Z", m, &[]).await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 2.0, "the deleted file's 3 is gone");
        assert_eq!(facts[0].path.as_deref(), Some("b.rs"));
    }

    #[tokio::test]
    async fn per_path_fold_keeps_unchanged_files_from_the_baseline() {
        // The incrementality guarantee: a file never rescanned since the baseline
        // keeps contributing. This is what makes delta captures correct.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;

        // Only a.rs is rescanned, now 10. b.rs (untouched) keeps its 2.
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            2,
            "2026-06-30T11:00:00.000000Z",
            m,
            &[("a.rs", 10.0)],
        )
        .await;

        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            12.0
        );
    }

    /// A `scan_kind = 'full'` capture by `producer` anchored to `snap_id` —
    /// what the baseline sweep records (tsk71): its scanned set is the
    /// RECONSTRUCTED tree as-of that snapshot, not the snapshot's own rows.
    async fn full_capture(
        store: &SqliteFactStore,
        producer: &str,
        snap_id: i64,
        captured_at: &str,
        measure_id: i64,
        facts: &[(&str, f64)],
    ) -> i64 {
        let mut capture = NewMetricCapture::done(1, producer, format!("metric:{producer}"));
        capture.snapshot_id = Some(snap_id);
        capture.captured_at = Some(at(captured_at));
        capture.scan_kind = "full".into();
        let rows: Vec<NewFact> = facts
            .iter()
            .map(|(path, value)| NewFact {
                subject_kind: Some("file".into()),
                subject_ref: Some((*path).to_string()),
                path: Some((*path).to_string()),
                ..NewFact::new(measure_id, *value)
            })
            .collect();
        store.record_facts(capture, rows).await.unwrap()
    }

    #[tokio::test]
    async fn full_scan_capture_supersedes_the_whole_reconstructed_tree() {
        // The tsk71 baseline: a full scan anchored to a DELTA snapshot must
        // supersede every path in the reconstructed tree at that snapshot —
        // not just the delta's own rows. b.rs dropped to 0 (no fact emitted);
        // it is NOT in snapshot 2's rows, but it IS in the reconstructed tree,
        // so the full capture supersedes it.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;

        // A delta snapshot listing ONLY a.rs; the baseline runs over it.
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        full_capture(
            &store,
            "g",
            2,
            "2026-06-30T11:00:00.000000Z",
            m,
            &[("a.rs", 1.0)],
        )
        .await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(
            total(&facts),
            1.0,
            "the full scan restates the whole tree: b.rs's stale 2 must be gone"
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].path.as_deref(), Some("a.rs"));
    }

    #[tokio::test]
    async fn full_scan_capture_excludes_deleted_paths_from_the_reconstruction() {
        // Reconstruction semantics match `tree_at`: a path whose latest row
        // (<= the anchor snapshot) is a tombstone is out of the tree, so the
        // full capture supersedes-to-nothing rather than resurrecting it.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;

        // b.rs deleted; the baseline then runs anchored to snapshot 2.
        snapshot_with(&store, 2, &[("b.rs", "deleted")]).await;
        full_capture(
            &store,
            "g",
            2,
            "2026-06-30T11:00:00.000000Z",
            m,
            &[("a.rs", 1.0)],
        )
        .await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 1.0, "a rescanned to 1; deleted b gone");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].path.as_deref(), Some("a.rs"));
    }

    #[tokio::test]
    async fn asserted_capture_with_snapshot_restates_only_its_emitted_paths() {
        // tsk72 direction: `record_metric` captures now carry a snapshot for
        // PROVENANCE — but their scanned set stays "exactly what I emitted".
        // If the snapshot were treated as a delta scanned set, this assertion
        // over snapshot 1 (which lists b.rs) would wipe b.rs's gauge fact.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;

        let mut capture = NewMetricCapture::done(1, "g", "agent");
        capture.snapshot_id = Some(1);
        capture.captured_at = Some(at("2026-06-30T11:00:00.000000Z"));
        capture.scan_kind = "asserted".into();
        let rows = vec![NewFact {
            subject_kind: Some("file".into()),
            subject_ref: Some("a.rs".into()),
            path: Some("a.rs".into()),
            ..NewFact::new(m, 7.0)
        }];
        store.record_facts(capture, rows).await.unwrap();

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 9.0, "a.rs updated to 7, b.rs's 2 untouched");
    }

    #[tokio::test]
    async fn snapshotless_capture_is_coerced_to_asserted() {
        // delta/full semantics need a snapshot to anchor on; a snapshot-less
        // capture can only be an assertion. The insert coerces so no caller
        // can accidentally record an unanchorable scan.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;
        let capture = NewMetricCapture::done(1, "g", "agent"); // scan_kind: delta, no snapshot
        let rows = vec![NewFact {
            path: Some("a.rs".into()),
            ..NewFact::new(m, 4.0)
        }];
        store.record_facts(capture, rows).await.unwrap();
        // Behaves as an assertion: its emitted path is its scanned set.
        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 4.0);
    }

    #[tokio::test]
    async fn has_full_capture_tracks_baseline_state_per_producer_and_version() {
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;

        // Delta capture alone doesn't count as a baseline.
        gauge_capture(&store, "g", 1, "2026-06-30T10:00:00.000000Z", m, &[]).await;
        assert!(!store.has_full_capture("g", 1, Some("v1")).await.unwrap());

        // A full capture at v1 counts — for v1 (and for "any version").
        let mut capture = NewMetricCapture::done(1, "g", "metric:g");
        capture.snapshot_id = Some(1);
        capture.scan_kind = "full".into();
        capture.producer_version = Some("v1".into());
        store.record_facts(capture, Vec::new()).await.unwrap();
        assert!(store.has_full_capture("g", 1, Some("v1")).await.unwrap());
        assert!(store.has_full_capture("g", 1, None).await.unwrap());
        // …but not for a different fingerprint (script changed → re-baseline).
        assert!(!store.has_full_capture("g", 1, Some("v2")).await.unwrap());
    }

    #[tokio::test]
    async fn scanned_paths_for_full_capture_cover_the_reconstructed_tree() {
        // The series fold's eviction set: a full capture scans the whole
        // reconstructed tree (incl. tombstones — a deletion is a scan result).
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;
        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        snapshot_with(&store, 2, &[("b.rs", "deleted")]).await;
        let cid = full_capture(&store, "g", 2, "2026-06-30T11:00:00.000000Z", m, &[]).await;

        let mut paths: Vec<String> = store
            .scanned_paths_for_captures(vec![cid])
            .await
            .unwrap()
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    #[tokio::test]
    async fn prune_drops_tree_captures_dominated_by_the_latest_full() {
        // tsk75: every effort-less tree capture strictly OLDER than the latest
        // done `full` capture of its (stream, producer) is dead weight — the
        // baseline restates every path it scanned, newer. Their facts were 69%
        // of a 778k-row fact table. Pruning must keep: the latest full, deltas
        // NEWER than it, effort-stamped captures (attribution history), other
        // producers without a full capture, and any capture carrying facts of
        // a non-per-path measure. The fold value must not move.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        snapshot_with(&store, 3, &[("b.rs", "oxplow")]).await;

        // Old delta + old full (both dominated), then the latest full, then a
        // newer delta refreshing a.rs.
        let old_delta = gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T08:00:00.000000Z",
            m,
            &[("a.rs", 9.0), ("b.rs", 9.0)],
        )
        .await;
        let old_full = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            m,
            &[("a.rs", 8.0), ("b.rs", 8.0)],
        )
        .await;
        let latest_full = full_capture(
            &store,
            "g",
            2,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;
        let new_delta = gauge_capture(
            &store,
            "g",
            2,
            "2026-06-30T11:00:00.000000Z",
            m,
            &[("a.rs", 10.0)],
        )
        .await;
        // An old effort-stamped capture — attribution history, never pruned.
        let mut stamped = NewMetricCapture::done(1, "g", "metric:g");
        stamped.snapshot_id = Some(1);
        stamped.captured_at = Some(at("2026-06-30T08:30:00.000000Z"));
        stamped.effort_id = Some(1);
        let stamped_id = store.record_facts(stamped, Vec::new()).await.unwrap();
        // A producer with no full capture — untouched.
        let other = gauge_capture(
            &store,
            "h",
            3,
            "2026-06-30T08:00:00.000000Z",
            m,
            &[("b.rs", 4.0)],
        )
        .await;

        let before = total(&store.latest_tree_facts(m, Some(1)).await.unwrap());
        let pruned = store.prune_dominated_tree_captures(1).await.unwrap();
        assert_eq!(pruned, 2, "old delta + old full dropped");

        let after = total(&store.latest_tree_facts(m, Some(1)).await.unwrap());
        assert_eq!(before, after, "the fold must not move");

        let alive: Vec<i64> = store
            .captures_for_producers(vec!["g".into(), "h".into()])
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(!alive.contains(&old_delta), "dominated delta pruned");
        assert!(!alive.contains(&old_full), "dominated full pruned");
        assert!(alive.contains(&latest_full));
        assert!(alive.contains(&new_delta));
        assert!(alive.contains(&stamped_id), "effort-stamped kept");
        assert!(alive.contains(&other), "producer without a baseline kept");
    }

    #[tokio::test]
    async fn a_prune_that_drops_captures_invalidates_that_streams_cube() {
        // tsk100. The prune deletes captures and their facts cascade.
        // `metric_live_fact` cascades with them (FK on `fact_id`), so the live
        // state self-heals — but `metric_cube` rows are FROZEN at build time and
        // do NOT. Leave the watermark standing and the read treats those stale
        // rows as authoritative: a wrong NUMBER, this subsystem's worst failure
        // mode.
        //
        // Usually the frozen rows happen to agree (a `full` sweep restates every
        // path it scanned, so a pruned capture's facts were already evicted at
        // every surviving point). But not always — a path live from before the
        // sweep that the sweep didn't restate (a changed gauge glob: neither
        // scanned nor tombstoned) is still live, and pruning deletes it. So
        // INVALIDATE rather than reason about which prunes are safe. The cube is
        // disposable; throwing it away costs only the next rebuild.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        let old_full = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            m,
            &[("a.rs", 8.0)],
        )
        .await;
        let latest_full = full_capture(
            &store,
            "g",
            2,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0)],
        )
        .await;

        // Stand in for a built cube over that history.
        let row = |v: f64| NewCubeRow {
            producer: "g".into(),
            dims_key: "{}".into(),
            fact_count: 1,
            value_sum: v,
            value_min: Some(v),
            value_max: Some(v),
            numerator: 0.0,
            denominator: 0.0,
        };
        for (cap, at_s, v) in [
            (old_full, "2026-06-30T09:00:00.000000Z", 8.0),
            (latest_full, "2026-06-30T10:00:00.000000Z", 3.0),
        ] {
            store
                .write_cube_rows(
                    m,
                    1,
                    None,
                    cap,
                    at(at_s),
                    vec![row(v)],
                    store.cube_epoch().await.unwrap(),
                )
                .await
                .unwrap();
        }
        assert!(store.cube_watermark(m, 1).await.unwrap().is_some());

        let pruned = store.prune_dominated_tree_captures(1).await.unwrap();
        assert_eq!(pruned, 1, "the dominated full is dropped");
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_none(),
            "a prune that dropped captures must invalidate the cube — an un-advanced \
             watermark reads as `not cubed yet` and sends the read back to the facts, \
             which are always right"
        );
        let rows = store.cube_rows_for_measure(m, Some(1)).await.unwrap();
        assert!(
            rows.is_empty(),
            "stale cube rows must not survive the prune"
        );
    }

    #[tokio::test]
    async fn a_prune_that_drops_nothing_leaves_the_cube_alone() {
        // The other half, and NOT hypothetical: `rebuild_metric_baseline` prunes on
        // EVERY boot (the "nothing to baseline" path). Invalidating unconditionally
        // would wipe a healthy cube each start and force a full re-fold — turning
        // tsk96's fix back off at boot, for nothing. Only a prune that actually
        // deleted something may invalidate.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        let only_full = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            m,
            &[("a.rs", 8.0)],
        )
        .await;
        store
            .write_cube_rows(
                m,
                1,
                None,
                only_full,
                at("2026-06-30T09:00:00.000000Z"),
                vec![NewCubeRow {
                    producer: "g".into(),
                    dims_key: "{}".into(),
                    fact_count: 1,
                    value_sum: 8.0,
                    value_min: Some(8.0),
                    value_max: Some(8.0),
                    numerator: 0.0,
                    denominator: 0.0,
                }],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();

        let pruned = store.prune_dominated_tree_captures(1).await.unwrap();
        assert_eq!(pruned, 0, "nothing is dominated");
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_some(),
            "a no-op prune must leave the cube built — else every boot pays a re-fold"
        );
        assert_eq!(
            store.cube_rows_for_measure(m, Some(1)).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn prune_keeps_captures_carrying_non_per_path_facts() {
        // A capture with any fact on a complete/per-subject measure holds real
        // history (their folds read past captures) — never prune it, even when
        // a same-producer full capture is newer.
        let store = fixture().await;
        let per_path = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        let complete = measure(&store, "acme.level").await; // default: complete
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;

        let mut mixed = NewMetricCapture::done(1, "g", "metric:g");
        mixed.snapshot_id = Some(1);
        mixed.captured_at = Some(at("2026-06-30T08:00:00.000000Z"));
        let mixed_id = store
            .record_facts(
                mixed,
                vec![NewFact {
                    path: Some("a.rs".into()),
                    ..NewFact::new(complete, 5.0)
                }],
            )
            .await
            .unwrap();
        full_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            per_path,
            &[("a.rs", 1.0)],
        )
        .await;

        let pruned = store.prune_dominated_tree_captures(1).await.unwrap();
        assert_eq!(pruned, 0, "mixed-measure capture must survive");
        let alive: Vec<i64> = store
            .captures_for_producers(vec!["g".into()])
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(alive.contains(&mixed_id));
    }

    #[tokio::test]
    async fn per_path_fold_partitions_by_producer_so_gauges_dont_supersede_each_other() {
        // The 10 idiom gauges all emit on `oxplow.ast_hit`, sliced by `rule`. If the
        // fold didn't partition by producer, gauge `g2`'s capture on the same
        // snapshot would supersede gauge `g1`'s facts for the same path.
        let store = fixture().await;
        let m = measure(&store, "oxplow.ast_hit").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g1",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0)],
        )
        .await;
        gauge_capture(
            &store,
            "g2",
            1,
            "2026-06-30T10:00:01.000000Z",
            m,
            &[("a.rs", 4.0)],
        )
        .await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 7.0, "both gauges' facts survive");
        assert_eq!(facts.len(), 2);
    }

    #[tokio::test]
    async fn aged_pruning_keeps_everything_a_current_value_stands_on() {
        // tsk93. `metricRetentionDays` is OPT-IN (default 0 = this never
        // runs); when enabled it may delete only history no current value
        // stands on: effort-stamped captures are attribution (kept), each
        // producer's newest capture anchors the headline/zero-fill (kept),
        // and any capture owning a latest-per-partition fact is LIVE in the
        // fold (kept) — deleting it would move today's number, which
        // retention must never do. Old, superseded, unstamped history is what
        // goes; its points vanish from the series (that IS retention), and
        // the affected stream's cube is invalidated + the epoch fenced.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.case", "acme.case")
            })
            .await
            .unwrap();
        let cap = |ts: &str| NewMetricCapture {
            captured_at: Some(at(ts)),
            ..NewMetricCapture::done(1, "tests", "builtin")
        };
        let subject = |s: &str, v: f64| NewFact {
            subject_ref: Some(s.into()),
            ..NewFact::new(m, v)
        };
        // c1 OLD: A=1, later superseded → the one deletable capture.
        let c1 = store
            .record_facts(cap("2026-06-01T10:00:00.000000Z"), vec![subject("A", 1.0)])
            .await
            .unwrap();
        // c2 OLD: B=2, never re-run → LIVE (latest B) → kept.
        let c2 = store
            .record_facts(cap("2026-06-01T11:00:00.000000Z"), vec![subject("B", 2.0)])
            .await
            .unwrap();
        // c3 OLD: A=3 supersedes c1 → LIVE (latest A) → kept.
        let c3 = store
            .record_facts(cap("2026-06-01T12:00:00.000000Z"), vec![subject("A", 3.0)])
            .await
            .unwrap();
        // c4 OLD, superseded, but EFFORT-STAMPED → attribution → kept.
        let c4 = store
            .record_facts(
                NewMetricCapture {
                    effort_id: Some(1),
                    ..cap("2026-06-01T09:00:00.000000Z")
                },
                vec![subject("A", 0.5)],
            )
            .await
            .unwrap();
        // c5 NEW (inside the window, its own subject so c3 stays A's latest)
        // → kept; also the producer's newest capture.
        let c5 = store
            .record_facts(cap("2026-06-30T10:00:00.000000Z"), vec![subject("C", 4.0)])
            .await
            .unwrap();
        // A stand-in cube build so invalidation is observable.
        store
            .write_cube_rows(
                m,
                1,
                None,
                c5,
                at("2026-06-30T10:00:00.000000Z"),
                vec![NewCubeRow {
                    producer: "tests".into(),
                    dims_key: "{}".into(),
                    fact_count: 1,
                    value_sum: 4.0,
                    value_min: Some(4.0),
                    value_max: Some(4.0),
                    numerator: 0.0,
                    denominator: 0.0,
                }],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();
        let epoch_before = store.cube_epoch().await.unwrap();

        let pruned = store
            .prune_aged_captures(at("2026-06-15T00:00:00.000000Z"))
            .await
            .unwrap();
        assert_eq!(pruned, 1, "exactly c1 — old, superseded, unstamped");

        let alive: Vec<i64> = store
            .captures_for_producers(vec!["tests".into()])
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(!alive.contains(&c1), "superseded old history pruned");
        assert!(
            alive.contains(&c2),
            "a still-live subject keeps its capture"
        );
        assert!(alive.contains(&c3), "the superseding capture is live");
        assert!(alive.contains(&c4), "effort-stamped = attribution, kept");
        assert!(alive.contains(&c5), "inside the window, kept");
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_none(),
            "the prune changed replay inputs — the stream's cube must go"
        );
        assert!(
            store.cube_epoch().await.unwrap() > epoch_before,
            "and any in-flight build must be fenced"
        );

        // A second pass deletes nothing and must leave the (rebuilt) cube
        // alone — this runs daily once enabled.
        assert_eq!(
            store
                .prune_aged_captures(at("2026-06-15T00:00:00.000000Z"))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store.cube_epoch().await.unwrap(),
            epoch_before + 1,
            "a no-op pass must not re-fence"
        );
    }

    #[tokio::test]
    async fn a_stale_epoch_batch_lands_nothing() {
        // tsk113. The batch's whole point is one transaction per chunk — and
        // the fence must hold at that granularity: an invalidation after the
        // builder planned the chunk means NONE of it may land — not the live
        // applies, not the rows, not the watermark.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.case", "acme.case")
            })
            .await
            .unwrap();
        let cap = store
            .record_facts(
                NewMetricCapture {
                    captured_at: Some(at("2026-06-30T10:00:00.000000Z")),
                    ..NewMetricCapture::done(1, "tests", "builtin")
                },
                vec![NewFact {
                    subject_ref: Some("T".into()),
                    ..NewFact::new(m, 1.0)
                }],
            )
            .await
            .unwrap();
        let step = |fact_id: i64| {
            (
                Some(BatchApply {
                    branch: None,
                    producer: "tests".into(),
                    restated: vec!["T".into()],
                    inserted: vec![("T".into(), fact_id)],
                }),
                BatchRows {
                    branch: None,
                    capture_id: cap,
                    captured_at: at("2026-06-30T10:00:00.000000Z"),
                    rows: vec![NewCubeRow {
                        producer: "tests".into(),
                        dims_key: "{}".into(),
                        fact_count: 1,
                        value_sum: 1.0,
                        value_min: Some(1.0),
                        value_max: Some(1.0),
                        numerator: 0.0,
                        denominator: 0.0,
                    }],
                },
            )
        };
        let fact_id: i64 = 1;
        let planned = store.cube_epoch().await.unwrap();
        // An invalidation lands after planning (a dim flip is one).
        store
            .upsert_dimension(NewDimension {
                promoted: true,
                ..NewDimension::categorical("acme.kind", "Kind")
            })
            .await
            .unwrap();
        assert!(
            !store
                .apply_build_batch(m, 1, vec![step(fact_id)], planned)
                .await
                .unwrap(),
            "the stale chunk must refuse"
        );
        assert!(store.cube_watermark(m, 1).await.unwrap().is_none());
        assert!(store.live_facts(m, 1, None).await.unwrap().is_empty());
        // The fresh epoch commits the same chunk.
        let fresh = store.cube_epoch().await.unwrap();
        assert!(store
            .apply_build_batch(m, 1, vec![step(fact_id)], fresh)
            .await
            .unwrap());
        assert!(store.cube_watermark(m, 1).await.unwrap().is_some());
        assert_eq!(store.live_facts(m, 1, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_stale_build_write_is_fenced_after_an_invalidation() {
        // tsk103 review. The build runs outside the prune's transaction, so a
        // wipe can land MID pass: the builder's todo-list predates it, and its
        // next write would re-plant a watermark covering captures whose rows
        // the wipe deleted — "covered but rowless", served as explicit 0s.
        // Every invalidation bumps the epoch; a write carrying the stale epoch
        // must refuse and write NOTHING.
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            m,
            &[("a.rs", 8.0)],
        )
        .await;
        let latest = full_capture(
            &store,
            "g",
            2,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0)],
        )
        .await;
        let built = || NewCubeRow {
            producer: "g".into(),
            dims_key: "{}".into(),
            fact_count: 1,
            value_sum: 3.0,
            value_min: Some(3.0),
            value_max: Some(3.0),
            numerator: 0.0,
            denominator: 0.0,
        };
        // The builder plans its pass (reads the epoch)…
        let planned_epoch = store.cube_epoch().await.unwrap();
        // …then the prune drops a capture and wipes+fences.
        let pruned = store.prune_dominated_tree_captures(1).await.unwrap();
        assert_eq!(pruned, 1);
        // The stale write must refuse.
        let written = store
            .write_cube_rows(
                m,
                1,
                None,
                latest,
                at("2026-06-30T10:00:00.000000Z"),
                vec![built()],
                planned_epoch,
            )
            .await
            .unwrap();
        assert!(!written, "a write planned before the wipe must be fenced");
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_none(),
            "the fenced write planted nothing — no watermark over rowless captures"
        );
        // A fresh pass (current epoch) writes normally.
        let fresh = store.cube_epoch().await.unwrap();
        assert!(fresh > planned_epoch, "the wipe bumped the epoch");
        assert!(store
            .write_cube_rows(
                m,
                1,
                None,
                latest,
                at("2026-06-30T10:00:00.000000Z"),
                vec![built()],
                fresh,
            )
            .await
            .unwrap());
        assert!(store.cube_watermark(m, 1).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn flipping_a_dims_promoted_bit_invalidates_the_cube() {
        // tsk103 review. Promotion changes the cube's GRAIN (`dims_key`), and
        // V64's rule is that no old-grain row may survive to be served — a
        // pre-promotion bucket lacks the new key and reads as an explicit 0
        // through a dim filter. The migrations honor that by hand; the CONFIG
        // path (`seed_catalog` → upsert_dimension, which runs EVERY boot) must
        // honor it automatically — and only on an actual FLIP, or every boot
        // wipes a healthy cube and turns tsk96's fix back off (the same
        // lesson tsk100 learned for the boot prune).
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        let cap = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            m,
            &[("a.rs", 8.0)],
        )
        .await;
        let built = |v: f64| NewCubeRow {
            producer: "g".into(),
            dims_key: "{}".into(),
            fact_count: 1,
            value_sum: v,
            value_min: Some(v),
            value_max: Some(v),
            numerator: 0.0,
            denominator: 0.0,
        };
        let dim = |promoted: bool| NewDimension {
            promoted,
            ..NewDimension::categorical("acme.kind", "Kind")
        };

        store
            .write_cube_rows(
                m,
                1,
                None,
                cap,
                at("2026-06-30T09:00:00.000000Z"),
                vec![built(8.0)],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();
        // The every-boot reseed with an UNCHANGED flag must leave the cube alone.
        store.upsert_dimension(dim(false)).await.unwrap();
        store.upsert_dimension(dim(false)).await.unwrap();
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_some(),
            "an unchanged promoted bit is the every-boot case — no wipe"
        );
        // Flipping it ON is a grain change: the whole cube must go.
        store.upsert_dimension(dim(true)).await.unwrap();
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_none(),
            "promoting a dim by config must invalidate the cube — old-grain \
             buckets would serve explicit 0s through the newly-eligible filter"
        );
        // And OFF again after a rebuild stand-in: also a grain change.
        store
            .write_cube_rows(
                m,
                1,
                None,
                cap,
                at("2026-06-30T09:00:00.000000Z"),
                vec![built(8.0)],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();
        store.upsert_dimension(dim(false)).await.unwrap();
        assert!(
            store.cube_watermark(m, 1).await.unwrap().is_none(),
            "demotion is a grain change too"
        );
    }

    #[tokio::test]
    async fn changing_a_measures_capture_scope_invalidates_only_that_measures_cube() {
        // tsk103 review. `capture_scope` picks the BUILD RULE (state fold vs
        // per-capture GROUP BY); rows built under the old rule must not be
        // served under the new one. Scoped to the one measure — and only on an
        // actual change, since `seed_catalog` re-upserts every measure at boot.
        let store = fixture().await;
        let a = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.a", "A")
            })
            .await
            .unwrap();
        let b = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.b", "B")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        let cap = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:00:00.000000Z",
            a,
            &[("a.rs", 8.0)],
        )
        .await;
        let built = || NewCubeRow {
            producer: "g".into(),
            dims_key: "{}".into(),
            fact_count: 1,
            value_sum: 8.0,
            value_min: Some(8.0),
            value_max: Some(8.0),
            numerator: 0.0,
            denominator: 0.0,
        };
        for mid in [a, b] {
            store
                .write_cube_rows(
                    mid,
                    1,
                    None,
                    cap,
                    at("2026-06-30T09:00:00.000000Z"),
                    vec![built()],
                    store.cube_epoch().await.unwrap(),
                )
                .await
                .unwrap();
        }
        // Same-scope re-upsert (every boot): untouched.
        store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.a", "A")
            })
            .await
            .unwrap();
        assert!(store.cube_watermark(a, 1).await.unwrap().is_some());
        // Scope change: A's cube goes, B's stays.
        store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.a", "A")
            })
            .await
            .unwrap();
        assert!(
            store.cube_watermark(a, 1).await.unwrap().is_none(),
            "a capture_scope change must invalidate that measure's cube"
        );
        assert!(
            store.cube_watermark(b, 1).await.unwrap().is_some(),
            "the other measure's cube is untouched"
        );
    }

    #[tokio::test]
    async fn non_done_captures_stay_out_of_the_fold_inputs() {
        // tsk103 review. The doc rule "non-done captures are invisible to
        // every fold" held for the three SQL folds but NOT for
        // `captures_for_producers`, which feeds the in-memory fold and the
        // cube build: a failed capture emitted a phantom point (repeating the
        // prior state at the failure's time) and, for a complete-scope
        // count/sum, would zero-splice — the "one failure zeroes the metric"
        // outcome the substrate promises against.
        let store = fixture().await;
        let m = measure(&store, "acme.case").await;
        store
            .record_facts(
                NewMetricCapture {
                    captured_at: Some(at("2026-06-30T10:00:00.000000Z")),
                    ..NewMetricCapture::done(1, "tests", "builtin")
                },
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        store
            .record_facts(
                NewMetricCapture {
                    captured_at: Some(at("2026-06-30T11:00:00.000000Z")),
                    status: "failed".into(),
                    ..NewMetricCapture::done(1, "tests", "builtin")
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let caps = store
            .captures_for_producers(vec!["tests".into()])
            .await
            .unwrap();
        assert_eq!(
            caps.len(),
            1,
            "only the done capture feeds the folds — a failed run is a \
             recorded event, never a data point"
        );
        assert_eq!(caps[0].status, "done");
    }

    #[tokio::test]
    async fn per_path_fold_supersedes_many_facts_per_path_wholesale() {
        // `todos.star` emits one fact PER MARKER (many facts share a path), and the
        // code gauges emit one fact per SYMBOL. Rescanning the file must replace the
        // whole set — so a removed marker/function disappears rather than lingering.
        let store = fixture().await;
        let m = measure(&store, "oxplow.todo").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 1.0), ("a.rs", 1.0), ("a.rs", 1.0)],
        )
        .await;
        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            3.0
        );

        // Two of the three TODOs are fixed.
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            2,
            "2026-06-30T11:00:00.000000Z",
            m,
            &[("a.rs", 1.0)],
        )
        .await;

        let facts = store.latest_tree_facts(m, Some(1)).await.unwrap();
        assert_eq!(
            total(&facts),
            1.0,
            "the old 3 facts are replaced, not added to"
        );
        assert_eq!(facts.len(), 1);
    }

    #[tokio::test]
    async fn per_path_fold_ignores_a_capture_that_scanned_nothing() {
        // An empty delta capture (nothing changed) restates NO paths, so it must
        // supersede nothing. Under the old semi-additive reading this was "the repo
        // is zero" — the bug in miniature.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0)],
        )
        .await;

        snapshot_with(&store, 2, &[]).await; // a snapshot with no files
        gauge_capture(&store, "g", 2, "2026-06-30T11:00:00.000000Z", m, &[]).await;

        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            3.0,
            "scanning nothing supersedes nothing"
        );
    }

    /// A test-run capture: one fact per case, `subject_ref = test:<name>`, value 1,
    /// status on the dims (what `record_test_run` writes).
    async fn test_run(
        store: &SqliteFactStore,
        captured_at: &str,
        measure_id: i64,
        cases: &[(&str, &str)],
    ) -> i64 {
        let mut capture = NewMetricCapture::done(1, "tests", "tests");
        capture.captured_at = Some(at(captured_at));
        let rows: Vec<NewFact> = cases
            .iter()
            .map(|(name, status)| NewFact {
                subject_kind: Some("test".into()),
                subject_ref: Some(format!("test:{name}")),
                dims_json: Some(format!("{{\"oxplow.status\":\"{status}\"}}")),
                ..NewFact::new(measure_id, 1.0)
            })
            .collect();
        store.record_facts(capture, rows).await.unwrap()
    }

    #[tokio::test]
    async fn per_subject_fold_survives_a_partial_test_run() {
        // tsk43. A FULL run knows 3 tests (one failing). Then someone runs a SINGLE
        // test file — a capture holding just that case. Read as `complete` ("the last
        // capture restates every test") the suite would shrink to 1 test and the
        // failure would vanish. Per-subject, the partial run updates only the test it
        // ran; the other two keep their last-known status.
        let store = fixture().await;
        let m = measure(&store, "oxplow.test_case").await;

        test_run(
            &store,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a", "passed"), ("b", "failed"), ("c", "passed")],
        )
        .await;
        let all = store.latest_subject_facts(m, Some(1)).await.unwrap();
        assert_eq!(all.len(), 3, "the full run knows 3 tests");

        // A partial run: only test `b`, now fixed.
        test_run(&store, "2026-06-30T11:00:00.000000Z", m, &[("b", "passed")]).await;

        let facts = store.latest_subject_facts(m, Some(1)).await.unwrap();
        assert_eq!(
            facts.len(),
            3,
            "the suite is still 3 tests — a partial run must not shrink it"
        );
        let failed = facts
            .iter()
            .filter(|f| {
                f.dims_json
                    .as_deref()
                    .is_some_and(|d| d.contains("\"failed\""))
            })
            .count();
        assert_eq!(
            failed, 0,
            "b's latest status supersedes its earlier failure"
        );
    }

    #[tokio::test]
    async fn a_failed_capture_never_supersedes_good_facts() {
        // tsk47's footgun. Recording a gauge FAILURE durably (so it stops being an
        // invisible warn) means writing a capture with NO facts. On a FULL-TREE
        // snapshot that capture restates every path — so if the fold counted it, a
        // single timeout would supersede every fact and silently zero the metric,
        // which is far worse than the bug we're fixing. Non-`done` captures must be
        // invisible to the fold.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        gauge_capture(
            &store,
            "g",
            1,
            "2026-06-30T10:00:00.000000Z",
            m,
            &[("a.rs", 3.0), ("b.rs", 2.0)],
        )
        .await;
        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            5.0
        );

        // The gauge times out on the next full-tree scan: a failed, fact-less capture
        // whose snapshot covers BOTH files.
        snapshot_with(&store, 2, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        let mut failed = NewMetricCapture::done(1, "g", "metric:g");
        failed.status = "failed".into();
        failed.error = Some("sandbox budget exceeded".into());
        failed.snapshot_id = Some(2);
        failed.captured_at = Some(at("2026-06-30T11:00:00.000000Z"));
        store.record_facts(failed, Vec::new()).await.unwrap();

        assert_eq!(
            total(&store.latest_tree_facts(m, Some(1)).await.unwrap()),
            5.0,
            "a failed run must not supersede anything — the metric keeps its last good value"
        );
    }

    #[tokio::test]
    async fn per_subject_test_durations_sum_to_a_real_suite_total() {
        // tsk46. Durations are `per-subject` for the same reason statuses are: a
        // partial run must refresh only the timings it measured. Suite = a(100) +
        // b(200) + c(50) = 350ms. Re-run just `b`, now 20ms → the total must be
        // 100 + 20 + 50 = 170, NOT 20 ("the one test I just ran"), and the slowest
        // must fall to a's 100.
        let store = fixture().await;
        let m = measure(&store, "oxplow.test_duration").await;
        let run = |ms: &[(&str, f64)], at_s: &str| {
            let rows: Vec<NewFact> = ms
                .iter()
                .map(|(name, v)| NewFact {
                    subject_kind: Some("test".into()),
                    subject_ref: Some(format!("test:{name}")),
                    ..NewFact::new(m, *v)
                })
                .collect();
            let mut c = NewMetricCapture::done(1, "tests", "tests");
            c.captured_at = Some(at(at_s));
            (c, rows)
        };

        let (c1, r1) = run(
            &[("a", 100.0), ("b", 200.0), ("c", 50.0)],
            "2026-06-30T10:00:00.000000Z",
        );
        store.record_facts(c1, r1).await.unwrap();
        assert_eq!(
            total(&store.latest_subject_facts(m, Some(1)).await.unwrap()),
            350.0
        );

        let (c2, r2) = run(&[("b", 20.0)], "2026-06-30T11:00:00.000000Z");
        store.record_facts(c2, r2).await.unwrap();

        let facts = store.latest_subject_facts(m, Some(1)).await.unwrap();
        assert_eq!(total(&facts), 170.0, "a + c carry forward; b is refreshed");
        let slowest = facts.iter().map(|f| f.value).fold(f64::MIN, f64::max);
        assert_eq!(slowest, 100.0, "b is no longer the slowest");
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
        // The migrations seed 24 built-in measures (10 in V43 + oxplow.ast_hit in
        // V45 + turn/task_effort/nudge in V46 + oxplow.effort_test_outcome in V53 +
        // oxplow.test_duration in V57 + cache_tokens/cache_usage/effort_tokens in
        // V59 + effort_steering/effort_time_to_green in V60 + oxplow.token_waste
        // in V61 + coverage.branch/coverage.function in V68 + doc_coverage in
        // V69); this upsert added one more.
        assert_eq!(store.list_measures().await.unwrap().len(), 26);
    }

    #[tokio::test]
    async fn coverage_measure_is_semi_additive() {
        // tsk13 (V50): coverage is a level snapshot — a run replaces the last,
        // so it collapses to the latest capture, not a history-blended Σn/Σd.
        let store = fixture().await;
        let cov = store
            .get_measure("oxplow.coverage")
            .await
            .unwrap()
            .expect("coverage measure seeded");
        assert_eq!(cov.temporal_semantics, "semi-additive");
    }

    #[tokio::test]
    async fn dimensions_seeded_and_upsertable() {
        let store = fixture().await;
        // The migrations seed 11 built-in conformed dims (8 in V43 + oxplow.rule
        // in V45 + oxplow.token_kind in V46 + oxplow.tests_stat in V53).
        let seeded = store.list_dimensions().await.unwrap();
        assert!(seeded.iter().any(|d| d.key == "oxplow.language"));
        assert!(seeded.iter().any(|d| d.key == "oxplow.rule"));
        assert!(seeded.iter().any(|d| d.key == "oxplow.token_kind"));
        assert!(seeded.iter().any(|d| d.key == "oxplow.tests_stat"));
        assert_eq!(seeded.len(), 11);

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
    async fn delete_spec_removes_row_and_is_idempotent() {
        let store = fixture().await;
        store
            .upsert_spec(NewMetricSpec::base(
                "acme.hotspots",
                "Hotspots",
                "oxplow.complexity",
                "count",
            ))
            .await
            .unwrap();
        assert!(store.get_spec("acme.hotspots").await.unwrap().is_some());

        store.delete_spec("acme.hotspots").await.unwrap();
        assert!(store.get_spec("acme.hotspots").await.unwrap().is_none());
        // Deleting a missing key is a no-op, not an error.
        store.delete_spec("acme.hotspots").await.unwrap();
        store.delete_spec("never.existed").await.unwrap();
    }

    #[tokio::test]
    async fn measure_has_active_spec_tracks_the_spec_table() {
        let store = fixture().await;
        // No spec sources the measure yet — the producer gate is closed.
        assert!(!store
            .measure_has_active_spec("oxplow.complexity")
            .await
            .unwrap());

        let id = store
            .upsert_spec(NewMetricSpec::base(
                "acme.hotspots",
                "Hotspots",
                "oxplow.complexity",
                "count",
            ))
            .await
            .unwrap();
        assert!(id > 0);
        assert!(store
            .measure_has_active_spec("oxplow.complexity")
            .await
            .unwrap());
        // A different measure is still un-consumed.
        assert!(!store
            .measure_has_active_spec("oxplow.tokens")
            .await
            .unwrap());

        // Pruning the last consumer re-closes the gate.
        store.delete_spec("acme.hotspots").await.unwrap();
        assert!(!store
            .measure_has_active_spec("oxplow.complexity")
            .await
            .unwrap());
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
    async fn record_facts_is_idempotent_on_key() {
        // tsk14: a keyed capture re-recorded (a replayed report) is a no-op —
        // the existing id comes back and no facts double-insert.
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        let build = || {
            (
                NewMetricCapture {
                    idempotency_key: Some("coverage|abc1234||payload".into()),
                    ..NewMetricCapture::done(1, "coverage", "coverage-report")
                },
                vec![NewFact::new(m, 1.0), NewFact::new(m, 2.0)],
            )
        };

        let (c1, f1) = build();
        let id1 = store.record_facts(c1, f1).await.unwrap();
        let (c2, f2) = build();
        let id2 = store.record_facts(c2, f2).await.unwrap();
        assert_eq!(id1, id2, "same key returns the existing capture");
        assert_eq!(
            store.facts_for_measure(m).await.unwrap().len(),
            2,
            "replay must not double-insert facts"
        );

        // A different key inserts a fresh capture + facts.
        let (mut c3, f3) = build();
        c3.idempotency_key = Some("coverage|def5678||payload".into());
        let id3 = store.record_facts(c3, f3).await.unwrap();
        assert_ne!(id3, id1, "a new key is a new capture");
        assert_eq!(store.facts_for_measure(m).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn record_facts_without_key_always_inserts_fresh() {
        // Keyless captures (gauges, tokens) never dedupe — every run is a row.
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        let id1 = store
            .record_facts(
                NewMetricCapture::done(1, "metrics", "builtin"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        let id2 = store
            .record_facts(
                NewMetricCapture::done(1, "metrics", "builtin"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        assert_ne!(id1, id2);
        assert_eq!(store.facts_for_measure(m).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn captures_for_effort_returns_only_that_efforts_captures() {
        // The attribution-by-claim spine (T-D): an effort's captures are those
        // stamped with its effort_id, oldest first — not a time window.
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        // The fixture seeds effort 1; a `None`-effort capture (operational) must
        // be excluded, and an effort with no captures returns empty.
        for (effort, at_ts) in [
            (Some(1), "2026-06-30T10:00:00Z"),
            (Some(1), "2026-06-30T11:00:00Z"),
            (None, "2026-06-30T12:00:00Z"),
        ] {
            store
                .record_facts(
                    NewMetricCapture {
                        effort_id: effort,
                        captured_at: Some(at(at_ts)),
                        ..NewMetricCapture::done(1, "metrics", "builtin")
                    },
                    vec![NewFact::new(m, 1.0)],
                )
                .await
                .unwrap();
        }
        let caps = store.captures_for_effort(1).await.unwrap();
        assert_eq!(
            caps.len(),
            2,
            "only effort 1's captures (the None one excluded)"
        );
        assert!(caps.iter().all(|c| c.effort_id == Some(1)));
        // Oldest first.
        assert!(caps[0].captured_at <= caps[1].captured_at);
        // An effort with no captures returns empty (not a time-window match).
        assert_eq!(store.captures_for_effort(99).await.unwrap().len(), 0);
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
