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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};

// Timestamp <-> canonical string helpers (mirror `metric_store.rs`; kept local so
// the fact layer doesn't couple to the old module that will be deleted).

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

const MEASURE_COLS: &str = "id, key, title, unit, subject_kind, temporal_semantics, \
     capture_scope, scope, description, created_at, updated_at, extension";

/// How a scope is stored: an extension's `extension:<name>` is kept as
/// `scope = 'global'` plus the `extension` column (V84), because the scope
/// CHECK can't be widened without rebuilding tables whose rows facts
/// cascade from.
fn stored_scope(scope: &str) -> (&str, Option<&str>) {
    match scope.strip_prefix("extension:") {
        Some(name) => ("global", Some(name)),
        None => (scope, None),
    }
}

/// The inverse of [`stored_scope`].
fn read_scope(scope: String, extension: Option<String>) -> String {
    match extension {
        Some(name) => format!("extension:{name}"),
        None => scope,
    }
}

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
        scope: read_scope(row.get(7)?, row.get(11)?),
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
    /// Set for an entity dimension: `{view, expr, join?}` (V88).
    pub entity_json: Option<String>,
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
    /// Set for an entity dimension: `{view, expr, join?}` (V88).
    pub entity_json: Option<String>,
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
            entity_json: None,
        }
    }
}

const DIM_COLS: &str = "key, label, value_type, subject_kind, vocabulary_json, scope, promoted, \
     entity_json, extension";

fn row_to_dimension(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dimension> {
    Ok(Dimension {
        key: row.get(0)?,
        label: row.get(1)?,
        value_type: row.get(2)?,
        subject_kind: row.get(3)?,
        vocabulary_json: row.get(4)?,
        scope: read_scope(row.get(5)?, row.get(8)?),
        promoted: row.get::<_, i64>(6)? != 0,
        entity_json: row.get(7)?,
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
    /// Set for an entity metric: `{view, where?, time?, value?, aggregation}`
    /// (V88). Its `aggregation` column is then `sum` (one value per capture).
    pub entity_json: Option<String>,
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
    pub entity_json: Option<String>,
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
            entity_json: None,
        }
    }
}

const SPEC_COLS: &str = "id, key, title, unit, source_measure, aggregation, filter_json, \
     formula, sliceable_dims_json, direction, target, warn_at, fail_at, description, \
     category, language, scope, display_kind, created_at, updated_at, extension, entity_json";

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
        scope: read_scope(row.get(16)?, row.get(20)?),
        display_kind: row.get(17)?,
        entity_json: row.get(21)?,
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
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
    pub branch: Option<String>,
    pub captured_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// The verbatim per-run detail payload, as an envelope
    /// `{"kind": "<detail kind>", "payload": {…}}` (test suite/case tree,
    /// coverage per-file line-sets, analysis findings; T-E1, tsk48).
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
    /// The agent turn it was measured in (V157), when one produced it.
    pub turn_id: Option<i64>,
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
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
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
    /// See [`MetricCapture::turn_id`]: the turn the producer knows it was
    /// measured in (its causing event's anchor), never guessed here.
    pub turn_id: Option<i64>,
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
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            captured_at: None,
            ended_at: None,
            detail_json: None,
            idempotency_key: None,
            producer_version: None,
            scan_kind: "delta".into(),
            turn_id: None,
        }
    }
}

const CAPTURE_COLS: &str = "id, stream_id, thread_id, effort_id, producer, status, error, scope, \
     trigger, basis_ref, provenance, source, snapshot_id, closest_vcs_rev, vcs_rev_exact, \
     branch, captured_at, ended_at, detail_json, producer_version, scan_kind, turn_id";

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
        closest_vcs_rev: row.get(13)?,
        vcs_rev_exact: row.get::<_, i64>(14)? != 0,
        branch: row.get(15)?,
        captured_at: string_to_ts(&captured_at).map_err(ts_conv_err)?,
        ended_at: match ended_at {
            Some(s) => Some(string_to_ts(&s).map_err(ts_conv_err)?),
            None => None,
        },
        detail_json: row.get(18)?,
        producer_version: row.get(19)?,
        scan_kind: row.get(20)?,
        turn_id: row.get(21)?,
    })
}

/// The measure `key`, read on the caller's connection.
pub fn get_measure_tx(conn: &rusqlite::Connection, key: &str) -> rusqlite::Result<Option<Measure>> {
    let sql = format!("SELECT {MEASURE_COLS} FROM measure WHERE key = ?1");
    conn.prepare_cached(&sql)?
        .query_row(params![key], row_to_measure)
        .optional()
}

/// The metric spec `key`, read on the caller's connection.
pub fn get_spec_tx(conn: &rusqlite::Connection, key: &str) -> rusqlite::Result<Option<MetricSpec>> {
    let sql = format!("SELECT {SPEC_COLS} FROM metric_spec WHERE key = ?1");
    conn.query_row(&sql, params![key], row_to_spec).optional()
}

/// Insert a capture and its facts inside the caller's transaction; when
/// `log` is given, append the event it builds from the capture id. A
/// capture already recorded under its idempotency key writes nothing and
/// returns the existing id (the partial unique index is the true guard;
/// this read is the fast path on the serialized write connection). The
/// caller commits, then calls [`SqliteFactStore::facts_committed`].
/// A duration moves enough to be written again when it differs from the last
/// one written by more than this fraction of it… (tsk733)
pub const DURATION_MOVE_RATIO: f64 = 0.5;
/// …and by at least this many milliseconds (a 5 ms test that takes 12 ms
/// didn't get slower in any way a person reads).
pub const DURATION_MOVE_MIN_MS: f64 = 20.0;

/// One test case's result in a run (a JUnit case), as
/// [`SqliteFactStore::record_test_run`] takes it.
#[derive(Debug, Clone, PartialEq)]
pub struct TestCaseResult {
    /// `test:<classname>::<name>` — the fact's subject and the summary key.
    pub subject: String,
    /// `passed`, `failed` or `skipped`.
    pub status: String,
    pub time_ms: Option<f64>,
    /// The status fact's (and duration fact's) dims.
    pub dims_json: Option<String>,
}

/// One test's summary on a (stream, branch, producer): `v_test_case_stat`.
#[derive(Debug, Clone, PartialEq)]
pub struct TestCaseStat {
    pub subject: String,
    pub last_status: String,
    pub last_ms: Option<f64>,
    /// The last duration written as a fact (what the tolerance compares to).
    pub recorded_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub mean_ms: Option<f64>,
    pub runs: i64,
    pub failures: i64,
    pub flips: i64,
    pub last_failed_at: Option<String>,
    pub last_passed_at: Option<String>,
    pub last_run_id: Option<i64>,
}

/// Which of a case's facts a run writes, given the test's summary on its
/// branch (`None`: new there): `(status fact, duration fact)`. A failure
/// is always written; a pass or skip only when new or its status changed; a
/// duration when the status fact is written for a new test, or when it moved
/// past the tolerance from the last duration written.
pub fn test_case_writes(prev: Option<&TestCaseStat>, case: &TestCaseResult) -> (bool, bool) {
    let status = match prev {
        _ if case.status == "failed" => true,
        None => true,
        Some(p) => p.last_status != case.status,
    };
    let duration = match (case.time_ms, prev.and_then(|p| p.recorded_ms)) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(ms), Some(was)) => {
            let moved = (ms - was).abs();
            moved > DURATION_MOVE_RATIO * was.max(1.0) && moved >= DURATION_MOVE_MIN_MS
        }
    };
    (status, duration)
}

pub fn record_facts_tx(
    conn: &rusqlite::Connection,
    capture: &NewMetricCapture,
    facts: &[NewFact],
    log: Option<&CaptureEvent>,
) -> Result<i64, DomainError> {
    if let Some(key) = capture.idempotency_key.as_deref() {
        let existing: Option<i64> = conn
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
    for f in facts {
        check_dims_tx(conn, f.dims_json.as_deref())?;
    }
    let capture_id = insert_capture(conn, capture).map_err(map_sql_err)?;
    for f in facts {
        insert_fact(conn, f, capture_id).map_err(map_sql_err)?;
    }
    if let Some(log) = log {
        let env = (log.build)(capture_id);
        crate::event_log_store::append_unique_tx(conn, &log.vocabulary.current(), &env)?;
    }
    Ok(capture_id)
}

/// A fact's dimension keys have one name each — the namespaced one (tsk945):
/// a bare key is refused, naming the conformed `oxplow.<key>` when the
/// catalog declares it.
fn check_dims_tx(conn: &rusqlite::Connection, dims_json: Option<&str>) -> Result<(), DomainError> {
    let Some(serde_json::Value::Object(dims)) =
        dims_json.and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
    else {
        return Ok(());
    };
    if let Some(key) = dims.keys().find(|k| !k.contains('.')) {
        let conformed = format!("oxplow.{key}");
        let declared: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM dimension WHERE key = ?1",
                params![conformed],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql_err)?;
        return Err(DomainError::Invalid(match declared {
            Some(_) => format!(
                "dimension key `{key}` isn't namespaced: its conformed name is `{conformed}`"
            ),
            None => format!(
                "dimension key `{key}` isn't namespaced: namespace it as yours (`<namespace>.{key}`)"
            ),
        }));
    }
    Ok(())
}

/// The summaries of `(stream, branch, producer)`, by subject.
fn test_case_stats_tx(
    conn: &rusqlite::Connection,
    stream_id: i64,
    branch: &str,
    producer: &str,
) -> Result<std::collections::HashMap<String, TestCaseStat>, DomainError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT subject, last_status, last_ms, recorded_ms, max_ms,
                    CASE WHEN timed_runs > 0 THEN total_ms / timed_runs END,
                    runs, failures, flips, last_failed_at, last_passed_at, last_run_id
               FROM test_case_stat
              WHERE stream_id = ?1 AND branch = ?2 AND producer = ?3",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![stream_id, branch, producer], |r| {
            Ok(TestCaseStat {
                subject: r.get(0)?,
                last_status: r.get(1)?,
                last_ms: r.get(2)?,
                recorded_ms: r.get(3)?,
                max_ms: r.get(4)?,
                mean_ms: r.get(5)?,
                runs: r.get(6)?,
                failures: r.get(7)?,
                flips: r.get(8)?,
                last_failed_at: r.get(9)?,
                last_passed_at: r.get(10)?,
                last_run_id: r.get(11)?,
            })
        })
        .map_err(map_sql_err)?;
    rows.map(|r| r.map(|s| (s.subject.clone(), s)))
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sql_err)
}

fn insert_capture(conn: &rusqlite::Connection, c: &NewMetricCapture) -> rusqlite::Result<i64> {
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
            provenance, source, snapshot_id, closest_vcs_rev, vcs_rev_exact, branch,
            captured_at, ended_at, detail_json, idempotency_key, producer_version, scan_kind,
            turn_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
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
            c.closest_vcs_rev,
            c.vcs_rev_exact,
            c.branch,
            captured,
            ended,
            c.detail_json,
            c.idempotency_key,
            c.producer_version,
            scan_kind,
            c.turn_id,
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
    pub closest_vcs_rev: Option<String>,
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
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
    pub basis_ref: Option<String>,
    pub snapshot_id: Option<i64>,
    pub stream_id: i64,
    pub thread_id: Option<i64>,
    pub effort_id: Option<i64>,
    /// The task `effort_id` belongs to (resolved per read from `effort`).
    pub task_id: Option<i64>,
    pub provenance: String,
    pub source: String,
    /// The capture's producer (gauge key / ingest kind) — identifies which scan
    /// emitted the fact, so reads can zero-fill a producer's EMPTY captures and
    /// scope "latest scan" currency per (stream, producer) (tsk44).
    pub producer: String,
}

/// The identity of a fact SLICE: the tuple the zero-splice producer discovery
/// groups by (`SqliteFactStore::distinct_slice_keys`, tsk239). Every fact in a
/// slice agrees on all four fields, so a predicate reading only these is
/// decided by the slice alone — no representative row needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSliceKey {
    pub producer: String,
    pub rule: Option<String>,
    pub severity: Option<String>,
    pub dims_json: Option<String>,
}

const FACT_ROW_COLS: &str = "f.id, f.capture_id, f.measure_id, f.value, f.numerator, \
     f.denominator, f.subject_kind, f.subject_ref, f.path, f.line, f.severity, f.rule, \
     f.detail, f.dims_json, c.captured_at, c.branch, c.closest_vcs_rev, \
     c.vcs_rev_exact, c.basis_ref, c.snapshot_id, c.stream_id, c.thread_id, \
     c.effort_id, c.provenance, c.source, c.producer";

/// A [`FactRow`] mapper that decodes each capture's `captured_at` **once**
/// (tsk215).
///
/// `captured_at` belongs to the fact's CAPTURE, so every fact in a capture
/// repeats it — ~130 facts per capture here (1.78M facts over 13.4k captures).
/// Decoding per row meant ~130 `String` allocations plus ~130 RFC3339 parses of
/// the identical text, which made row decode the top oxplow-owned cost in the
/// tsk208 profile once `dim_value` was fixed.
///
/// The memo keys on `capture_id`, and `capture_id -> captured_at` is a function,
/// so a hit is correct **regardless of row order**. Ordering (the queries sort by
/// `captured_at, id`) only decides the hit RATE — never correctness — so no
/// caller has to guarantee adjacency.
fn fact_row_mapper(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<FactRow>> {
    // Effort → task, loaded once per read: `effort` is small next to the
    // facts, and a per-row join would cost a lookup on every fact.
    let tasks: std::collections::HashMap<i64, i64> = conn
        .prepare_cached("SELECT id, task_id FROM v_effort WHERE task_id IS NOT NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut last: Option<(i64, Timestamp)> = None;
    Ok(move |row: &rusqlite::Row<'_>| {
        let capture_id: i64 = row.get(1)?;
        let captured_at = match last {
            Some((id, ts)) if id == capture_id => ts,
            _ => {
                // `get_ref` borrows the column; the `String` allocation only
                // happened to feed the parser.
                let raw = row.get_ref(14)?.as_str().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        14,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?;
                let ts = string_to_ts(raw).map_err(ts_conv_err)?;
                last = Some((capture_id, ts));
                ts
            }
        };
        let mut fact = row_to_fact_row_with(row, captured_at)?;
        fact.task_id = fact.effort_id.and_then(|e| tasks.get(&e).copied());
        Ok(fact)
    })
}

/// The column-by-column decode, with `captured_at` supplied by the caller so it
/// can be memoized per capture (see [`fact_row_mapper`]).
fn row_to_fact_row_with(
    row: &rusqlite::Row<'_>,
    captured_at: Timestamp,
) -> rusqlite::Result<FactRow> {
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
        captured_at,
        branch: row.get(15)?,
        closest_vcs_rev: row.get(16)?,
        vcs_rev_exact: row.get::<_, i64>(17)? != 0,
        basis_ref: row.get(18)?,
        snapshot_id: row.get(19)?,
        stream_id: row.get(20)?,
        thread_id: row.get(21)?,
        effort_id: row.get(22)?,
        task_id: None,
        provenance: row.get(23)?,
        source: row.get(24)?,
        producer: row.get(25)?,
    })
}

fn insert_fact(conn: &rusqlite::Connection, f: &NewFact, capture_id: i64) -> rusqlite::Result<i64> {
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

/// The cube read cache (tsk196): the rows served for one `cube_version`.
///
/// Keyed by `(measure_id, stream)` because `cube_rows_for_measure` filters on
/// `c.stream_id` — the same measure read for two streams is two answers. The
/// whole map is dropped when the version moves rather than evicted per entry:
/// a version change means every measure's rows may have shifted, and it bounds
/// the map to the measures actually read within a single version.
#[derive(Default)]
struct CubeRowsCache {
    version: i64,
    entries: HashMap<(i64, Option<i64>), Arc<Vec<CubeReadRow>>>,
}

impl CubeRowsCache {
    fn get(
        &self,
        version: i64,
        measure_id: i64,
        stream: Option<i64>,
    ) -> Option<&Arc<Vec<CubeReadRow>>> {
        if self.version != version {
            return None;
        }
        self.entries.get(&(measure_id, stream))
    }

    fn put(
        &mut self,
        version: i64,
        measure_id: i64,
        stream: Option<i64>,
        rows: Arc<Vec<CubeReadRow>>,
    ) {
        // A moved version invalidates EVERY measure, not just this one — the
        // trigger fires for any cube row, and we can't tell which measure it
        // belonged to. Dropping the map is both correct and what bounds it.
        if self.version != version {
            self.version = version;
            self.entries.clear();
        }
        self.entries.insert((measure_id, stream), rows);
    }
}

/// The event a capture write logs with it: built from the new capture's id
/// and validated against `schemas` on append.
pub struct CaptureEvent {
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub build: Box<dyn Fn(i64) -> oxplow_domain::Envelope + Send + Sync>,
}

#[derive(Clone)]
pub struct SqliteFactStore {
    db: Database,
    cube_rows: Arc<Mutex<CubeRowsCache>>,
}

impl SqliteFactStore {
    /// Why the ingest would refuse facts with these dimensions, if it
    /// would: the first bare key, named with its conformed one (tsk986). A
    /// producer asks before it records, so a refused run is a failed run
    /// rather than a capture rolled back behind one reported as recorded.
    pub async fn refused_dims(&self, dims: Vec<String>) -> Result<Option<String>, DomainError> {
        self.db
            .read(move |conn| {
                for d in &dims {
                    match check_dims_tx(conn, Some(d)) {
                        Ok(()) => {}
                        Err(DomainError::Invalid(why)) => return Ok(Some(why)),
                        Err(e) => return Err(e),
                    }
                }
                Ok(None)
            })
            .await
    }

    /// The database this store reads, for reads that go through the
    /// semantic layer instead (entity metrics, tsk322).
    pub fn database(&self) -> Database {
        self.db.clone()
    }

    pub fn new(db: Database) -> Self {
        Self {
            db,
            cube_rows: Arc::new(Mutex::new(CubeRowsCache::default())),
        }
    }

    // --- catalogs ---------------------------------------------------------

    /// Insert or update (by `key`) a measure; returns its row id. `created_at` is
    /// preserved across updates.
    pub async fn upsert_measure(&self, m: NewMeasure) -> Result<i64, DomainError> {
        self.db
            .transaction(move |tx| {
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
                let (scope, extension) = stored_scope(&m.scope);
                tx.execute(
                    "INSERT INTO measure
                       (key, title, unit, subject_kind, temporal_semantics, capture_scope,
                        scope, description, created_at, updated_at, extension)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)
                     ON CONFLICT(key) DO UPDATE SET
                        title=excluded.title, unit=excluded.unit,
                        subject_kind=excluded.subject_kind,
                        temporal_semantics=excluded.temporal_semantics,
                        capture_scope=excluded.capture_scope,
                        scope=excluded.scope, extension=excluded.extension,
                        description=excluded.description, updated_at=excluded.updated_at",
                    params![
                        m.key,
                        m.title,
                        m.unit,
                        m.subject_kind,
                        m.temporal_semantics,
                        m.capture_scope,
                        scope,
                        m.description,
                        now,
                        extension,
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
                Ok(id)
            })
            .await
    }

    pub async fn get_measure(&self, key: &str) -> Result<Option<Measure>, DomainError> {
        let key = key.to_string();
        self.db.call(move |conn| get_measure_tx(conn, &key)).await
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
            .transaction(move |tx| {
                let (scope, extension) = stored_scope(&d.scope);
                let prior: Option<bool> = tx
                    .query_row(
                        "SELECT promoted FROM dimension WHERE key = ?1",
                        params![d.key],
                        |r| r.get::<_, i64>(0).map(|v| v != 0),
                    )
                    .optional()
                    .map_err(map_sql_err)?;
                tx.execute(
                    "INSERT INTO dimension (key, label, value_type, subject_kind, vocabulary_json, scope, promoted, entity_json, extension)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(key) DO UPDATE SET
                        label=excluded.label, value_type=excluded.value_type,
                        subject_kind=excluded.subject_kind,
                        vocabulary_json=excluded.vocabulary_json, scope=excluded.scope,
                        promoted=excluded.promoted, entity_json=excluded.entity_json,
                        extension=excluded.extension",
                    params![d.key, d.label, d.value_type, d.subject_kind, d.vocabulary_json, scope, d.promoted, d.entity_json, extension],
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
                let (scope, extension) = stored_scope(&s.scope);
                conn.execute(
                    "INSERT INTO metric_spec
                       (key, title, unit, source_measure, aggregation, filter_json, formula,
                        sliceable_dims_json, direction, target, warn_at, fail_at, description,
                        category, language, scope, display_kind, created_at, updated_at, extension,
                        entity_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16, ?17, ?18, ?18, ?19, ?20)
                     ON CONFLICT(key) DO UPDATE SET
                        title=excluded.title, unit=excluded.unit,
                        source_measure=excluded.source_measure, aggregation=excluded.aggregation,
                        filter_json=excluded.filter_json, formula=excluded.formula,
                        sliceable_dims_json=excluded.sliceable_dims_json,
                        direction=excluded.direction, target=excluded.target,
                        warn_at=excluded.warn_at, fail_at=excluded.fail_at,
                        description=excluded.description, category=excluded.category,
                        language=excluded.language, scope=excluded.scope,
                        extension=excluded.extension, entity_json=excluded.entity_json,
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
                        scope,
                        s.display_kind,
                        now,
                        extension,
                        s.entity_json,
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
        self.db.call(move |conn| get_spec_tx(conn, &key)).await
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
        self.db.call(move |conn| insert_capture(conn, &c)).await
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
        self.record_facts_logged(capture, facts, None).await
    }

    /// [`Self::record_facts`], and when `log` is given, the event it builds
    /// from the new capture's id appended in the same transaction (P3.6:
    /// `test.run.recorded` commits with the run). A capture already recorded
    /// under its idempotency key writes nothing — no event either, the first
    /// write logged it.
    pub async fn record_facts_logged(
        &self,
        capture: NewMetricCapture,
        facts: Vec<NewFact>,
        log: Option<CaptureEvent>,
    ) -> Result<i64, DomainError> {
        let measures: std::collections::BTreeSet<i64> =
            facts.iter().map(|f| f.measure_id).collect();
        let result = self
            .db
            .transaction(move |tx| {
                let capture_id = record_facts_tx(tx, &capture, &facts, log.as_ref())?;
                Ok(capture_id)
            })
            .await;
        // New facts can introduce a producer, which is the one thing that
        // changes `producers_for_measure` — for their own measures only.
        // Invalidate AFTER the commit, and only on success — a rolled-back
        // write changed nothing.
        if result.is_ok() {
            self.db.memo().invalidate_measures(measures);
        }
        result
    }

    /// Record a test run with per-case results (tsk733): the capture, the
    /// per-case facts that say something new ([`test_case_writes`] against
    /// each test's summary on the capture's branch), and every case's
    /// summary row — one transaction. A replay under the capture's
    /// idempotency key returns the recorded run and changes nothing.
    pub async fn record_test_run(
        &self,
        capture: NewMetricCapture,
        cases: Vec<TestCaseResult>,
        case_measure: i64,
        duration_measure: Option<i64>,
        log: Option<CaptureEvent>,
    ) -> Result<i64, DomainError> {
        let result = self
            .db
            .transaction(move |tx| {
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
                let stream = capture.stream_id;
                let branch = capture.branch.clone().unwrap_or_default();
                let producer = capture.producer.clone();
                let when = capture.captured_at.unwrap_or_else(Timestamp::now);
                let at = ts_to_string(when);
                // Its own copy per attempt: the retried closure keeps the
                // input to try again with.
                let mut capture = capture.clone();
                capture.captured_at = Some(when);
                let prev = test_case_stats_tx(tx, stream, &branch, &producer)?;
                let mut facts = Vec::new();
                for case in &cases {
                    let (status, duration) = test_case_writes(prev.get(&case.subject), case);
                    if status {
                        facts.push(NewFact {
                            subject_kind: Some("test".into()),
                            subject_ref: Some(case.subject.clone()),
                            dims_json: case.dims_json.clone(),
                            ..NewFact::new(case_measure, 1.0)
                        });
                    }
                    if let (true, Some(measure), Some(ms)) = (duration, duration_measure, case.time_ms) {
                        facts.push(NewFact {
                            subject_kind: Some("test".into()),
                            subject_ref: Some(case.subject.clone()),
                            dims_json: case.dims_json.clone(),
                            ..NewFact::new(measure, ms)
                        });
                    }
                }
                let capture_id = record_facts_tx(tx, &capture, &facts, log.as_ref())?;
                let mut upsert = tx
                    .prepare_cached(
                        "INSERT INTO test_case_stat
                           (stream_id, branch, producer, subject, last_status, last_ms, recorded_ms,
                            max_ms, timed_runs, total_ms, runs, failures, flips, first_seen_at,
                            last_seen_at, last_failed_at, last_passed_at, last_run_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?8, coalesce(?6, 0), 1, ?9, 0, ?10,
                                 ?10, CASE WHEN ?5 = 'failed' THEN ?10 END,
                                 CASE WHEN ?5 = 'passed' THEN ?10 END, ?11)
                         ON CONFLICT (stream_id, branch, producer, subject) DO UPDATE SET
                           flips = flips + (last_status <> excluded.last_status),
                           last_status = excluded.last_status,
                           last_ms = coalesce(excluded.last_ms, last_ms),
                           recorded_ms = coalesce(excluded.recorded_ms, recorded_ms),
                           max_ms = CASE WHEN excluded.last_ms IS NULL THEN max_ms
                                         ELSE max(coalesce(max_ms, excluded.last_ms), excluded.last_ms) END,
                           timed_runs = timed_runs + excluded.timed_runs,
                           total_ms = total_ms + excluded.total_ms,
                           runs = runs + 1,
                           failures = failures + excluded.failures,
                           last_seen_at = excluded.last_seen_at,
                           last_failed_at = coalesce(excluded.last_failed_at, last_failed_at),
                           last_passed_at = coalesce(excluded.last_passed_at, last_passed_at),
                           last_run_id = excluded.last_run_id",
                    )
                    .map_err(map_sql_err)?;
                for case in &cases {
                    let (_, duration) = test_case_writes(prev.get(&case.subject), case);
                    let recorded = if duration { case.time_ms } else { None };
                    upsert
                        .execute(params![
                            stream,
                            branch,
                            producer,
                            case.subject,
                            case.status,
                            case.time_ms,
                            recorded,
                            i64::from(case.time_ms.is_some()),
                            i64::from(case.status == "failed"),
                            at,
                            capture_id,
                        ])
                        .map_err(map_sql_err)?;
                }
                drop(upsert);
                Ok(capture_id)
            })
            .await;
        if result.is_ok() {
            self.db
                .memo()
                .invalidate_measures([case_measure].into_iter().chain(duration_measure));
        }
        result
    }

    /// Every test's summary on `(stream, branch, producer)`.
    pub async fn test_case_stats(
        &self,
        stream_id: i64,
        branch: Option<String>,
        producer: &str,
    ) -> Result<Vec<TestCaseStat>, DomainError> {
        let branch = branch.unwrap_or_default();
        let producer = producer.to_string();
        self.db
            .call_mut(move |conn| {
                let mut out: Vec<TestCaseStat> =
                    test_case_stats_tx(conn, stream_id, &branch, &producer)?
                        .into_values()
                        .collect();
                out.sort_by(|a, b| a.subject.cmp(&b.subject));
                Ok(out)
            })
            .await
    }

    /// Forget the memoized fact reads of `measures` — after a caller's own
    /// transaction recorded facts for them through [`record_facts_tx`] and
    /// committed.
    pub fn facts_committed(&self, measures: impl IntoIterator<Item = i64>) {
        self.db.memo().invalidate_measures(measures);
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
                let rows = stmt.query_map(params![measure_id], fact_row_mapper(conn)?)?;
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
                let rows =
                    stmt.query_map(params![measure_id, stream_id], fact_row_mapper(conn)?)?;
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
    ///
    /// **Memoized** (tsk130). This was the single biggest backend CPU sink in
    /// the tsk129 profile — 309 s inclusive, ~46% of all backend CPU — not for
    /// want of an index (`idx_fact_measure_capture` covers it) but from call
    /// volume: it runs once per measure inside the cube build, the cube read
    /// fold, and the facts fallback, so every metric read pays it again for an
    /// answer that only changes when new facts land.
    ///
    /// The memo lives on [`Database`] rather than on this struct because the
    /// app builds several `SqliteFactStore`s over one `Database` — see
    /// [`crate::database::QueryMemo`]. Invalidation is a single wholesale clear
    /// in [`Self::record_facts`], which is the only path that inserts facts.
    pub async fn producers_for_measure(&self, measure_id: i64) -> Result<Vec<String>, DomainError> {
        let (generation, hit) = self.db.memo().producers_get(measure_id);
        if let Some(hit) = hit {
            return Ok(hit);
        }
        let producers = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT DISTINCT c.producer FROM metric_capture c
                      WHERE EXISTS (SELECT 1 FROM fact f
                                     WHERE f.capture_id = c.id AND f.measure_id = ?1)",
                )?;
                let rows = stmt.query_map(params![measure_id], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        // Declines to cache if a fact write landed while the query ran.
        self.db
            .memo()
            .producers_put(measure_id, generation, producers.clone());
        Ok(producers)
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

    /// Compact old per-run drill-in payloads: `NULL` out `detail_json` for
    /// captures beyond either retention bound. Returns the number compacted.
    ///
    /// **Compaction, not deletion (tsk211).** The capture row and every one of
    /// its facts survive, so no metric value, trend point, or count changes —
    /// only `list_effort_observations`' Tests/Coverage detail for old runs is
    /// lost. That is why this is safe to have ON by default, unlike
    /// `prune_aged_captures` (which deletes facts and stays opt-in).
    ///
    /// Two bounds, applied together — a capture is compacted if EITHER matches:
    /// - `max_per_producer`: keep detail only for the newest N captures of each
    ///   producer. The bound that actually holds a busy repo, where the payload
    ///   is ~0.5 MB per coverage run and age never catches up.
    /// - `older_than`: keep detail only for captures at/after this timestamp.
    ///   Reaches a project that has gone quiet, which a count cap never does.
    ///
    /// `None` disables that bound. Dedup is unaffected: `idempotency_key` is
    /// derived from `detail_json` at WRITE time and stored in its own column,
    /// so a replayed report still matches after the payload is gone.
    /// Checkpoint the WAL and truncate it back to zero (tsk216).
    ///
    /// The WAL grows to the high-water mark of the biggest write burst — a
    /// metric rebuild pushed it to 169MB here — and **never shrinks on its own**:
    /// SQLite's automatic checkpoint is PASSIVE, which copies frames into the
    /// main database and restarts the WAL in place, reusing the space rather
    /// than returning it. Measured on that 169MB file, only **226 frames were
    /// live**, so this is purely a disk-footprint fix; readers index live frames,
    /// not file bytes, so a stale WAL costs nothing to read.
    ///
    /// Best-effort by design: `TRUNCATE` needs every reader to have moved past
    /// the frames it wants to reclaim, so under concurrent readers it can return
    /// busy and reclaim nothing. That's fine — it runs again tomorrow.
    pub async fn checkpoint_wal(&self) -> Result<(), DomainError> {
        self.db
            .call(move |conn| {
                conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
                Ok(())
            })
            .await
    }

    pub async fn compact_capture_details(
        &self,
        older_than: Option<Timestamp>,
        max_per_producer: Option<u32>,
    ) -> Result<u64, DomainError> {
        if older_than.is_none() && max_per_producer.is_none() {
            return Ok(0);
        }
        let cutoff = older_than.map(ts_to_string);
        let keep = max_per_producer.map(|n| n as i64);
        self.db
            .call(move |conn| {
                // One statement, both bounds. `ROW_NUMBER` ranks each producer's
                // captures newest-first so the count cap is per producer rather
                // than global — a chatty producer must not evict a quiet one's
                // detail.
                let n = conn.execute(
                    "UPDATE metric_capture SET detail_json = NULL
                      WHERE detail_json IS NOT NULL
                        AND id IN (
                          SELECT id FROM (
                            SELECT id, captured_at,
                                   ROW_NUMBER() OVER (
                                     PARTITION BY producer
                                     ORDER BY captured_at DESC, id DESC
                                   ) AS rn
                              FROM metric_capture
                             WHERE detail_json IS NOT NULL
                          )
                          WHERE (?1 IS NOT NULL AND rn > ?1)
                             OR (?2 IS NOT NULL AND captured_at < ?2)
                        )",
                    params![keep, cutoff],
                )?;
                Ok(n as u64)
            })
            .await
    }

    /// `(capture count, newest capture id)` over `producers` — a cheap freshness
    /// token for a memoized read of those producers' series (tsk205).
    ///
    /// **Capture-scoped, deliberately not fact-scoped.** A fact-derived token
    /// (`MAX(fact.capture_id)`) is WRONG here: a rescan that finds the file clean
    /// emits NO fact and supersedes the old count with 0 (tsk41/tsk44), so the
    /// fact max never moves while the series changes. Counting the producers'
    /// captures sees those empty captures. `COUNT` as well as `MAX` so a delete
    /// that lowers the count still registers.
    ///
    /// Producer-scoped rather than global so the ~10s OTLP token captures (a
    /// different producer) don't invalidate a complexity/coverage series they
    /// cannot affect.
    pub async fn capture_token_for_producers(
        &self,
        producers: Vec<String>,
    ) -> Result<(i64, Option<i64>), DomainError> {
        if producers.is_empty() {
            return Ok((0, None));
        }
        self.db
            .call(move |conn| {
                let placeholders = (1..=producers.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT COUNT(*), MAX(id) FROM metric_capture
                      WHERE status = 'done' AND producer IN ({placeholders})"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params_from_iter(producers.iter()), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
                })
            })
            .await
    }

    /// The cube's read-cache token (V72, tsk196) — bumped by trigger on EVERY
    /// `metric_cube` insert/update/delete, cascades included.
    ///
    /// Distinct from [`Self::cube_epoch`], which fences concurrent WRITERS and
    /// deliberately does not move on an ordinary fold. This moves on every
    /// mutation, so a reader that saw version V is guaranteed the cube has not
    /// changed while V holds. See the V72 migration for why the two can't be
    /// one counter.
    pub async fn cube_version(&self) -> Result<i64, DomainError> {
        self.db
            .call(move |conn| {
                conn.prepare_cached("SELECT version FROM metric_cube_epoch WHERE id = 1")?
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
                let rows = stmt.query_map(
                    params![measure_id, stream_id, branch],
                    fact_row_mapper(conn)?,
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// A **per-subject** partition's seed: for each `(producer, subject)` among
    /// the facts of `capture_ids` (the captures visible to the branch), every
    /// fact of the LATEST such capture, as `(producer, subject_key, fact_id)`.
    /// The subject key is the fold key — `subject_ref`, else `path`, else
    /// `scalar_key` — so this is exactly what replaying those captures
    /// oldest-first (evict the keys a capture restates, insert its facts)
    /// leaves standing, computed over the index instead of by loading the
    /// history (tsk704: ten million per-case test facts took 25 s a seed).
    pub async fn live_seed_per_subject(
        &self,
        measure_id: i64,
        capture_ids: Vec<i64>,
        scalar_key: &str,
    ) -> Result<Vec<(String, String, i64)>, DomainError> {
        let ids = serde_json::to_string(&capture_ids).unwrap_or_else(|_| "[]".into());
        let scalar_key = scalar_key.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "WITH ranked AS (
                       SELECT c.producer,
                              COALESCE(f.subject_ref, f.path, ?3) AS subject_key,
                              f.id AS fact_id,
                              DENSE_RANK() OVER (
                                PARTITION BY c.producer, COALESCE(f.subject_ref, f.path, ?3)
                                ORDER BY c.captured_at DESC, c.id DESC
                              ) AS latest
                         FROM fact f
                         JOIN metric_capture c ON c.id = f.capture_id
                        WHERE f.measure_id = ?1
                          AND c.id IN (SELECT value FROM json_each(?2))
                     )
                     SELECT producer, subject_key, fact_id FROM ranked WHERE latest = 1
                      ORDER BY producer, subject_key, fact_id",
                )?;
                let rows = stmt.query_map(params![measure_id, ids, scalar_key], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
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
            .transaction(move |tx| {
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
            .transaction(move |tx| {
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
                Ok(true)
            })
            .await
    }

    /// Every cube bucket for a measure, oldest capture first, joined to its
    /// capture's spine — the read's replacement for decoding the raw facts.
    /// `stream` bounds it to one worktree; `None` reads every stream (each row
    /// still carries its own, so an unscoped read is a UNION, never a merge).
    ///
    /// Cached on [`Self::cube_version`] (tsk196). The read is a full scan of the
    /// measure's cube rows joined to `metric_capture` with a timestamp parse per
    /// row, and the UI fires it once per mounted tile on every
    /// `metricSamplesChanged` — so N tiles meant N identical scans. The version
    /// is trigger-maintained, so a hit is only ever served while the cube is
    /// provably unchanged; this trades memory for CPU, never freshness.
    pub async fn cube_rows_for_measure(
        &self,
        measure_id: i64,
        stream: Option<i64>,
    ) -> Result<Vec<CubeReadRow>, DomainError> {
        // Read the version FIRST: the query then observes a cube at least as
        // new as `version`. Caching newer rows under an older key is harmless
        // (the next reader sees the moved version and misses); the reverse —
        // caching older rows under a newer key — is what this ordering rules
        // out.
        let version = self.cube_version().await?;
        let hit = {
            let cache = self.cube_rows.lock().unwrap_or_else(|e| e.into_inner());
            cache.get(version, measure_id, stream).cloned()
        };
        if let Some(rows) = hit {
            return Ok(rows.as_ref().clone());
        }

        let rows = self
            .cube_rows_for_measure_uncached(measure_id, stream)
            .await?;
        self.cube_rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(version, measure_id, stream, Arc::new(rows.clone()));
        Ok(rows)
    }

    async fn cube_rows_for_measure_uncached(
        &self,
        measure_id: i64,
        stream: Option<i64>,
    ) -> Result<Vec<CubeReadRow>, DomainError> {
        self.db
            .call(move |conn| {
                let sql = "SELECT mc.producer, mc.dims_key, mc.fact_count, mc.value_sum,
                                  mc.value_min, mc.value_max, mc.numerator, mc.denominator,
                                  c.id, c.captured_at, c.stream_id, c.producer, c.branch,
                                  c.provenance, c.source, c.closest_vcs_rev
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
                        closest_vcs_rev: r.get(15)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// The PATH-LESS, SUBJECT-LESS facts of a measure — agent-asserted repo
    /// scalars (`metric.record` with no subject). The per-path read supplements
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
                let rows =
                    stmt.query_map(params![measure_id, stream_id], fact_row_mapper(conn)?)?;
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
                let placeholders = vec!["?"; keep.len()].join(", ");
                let sql = format!(
                    "DELETE FROM metric_spec
                      WHERE scope = 'project' AND key NOT IN ({placeholders})"
                );
                let n = conn.execute(&sql, rusqlite::params_from_iter(keep.iter()))?;
                Ok(n as u64)
            })
            .await
    }

    /// Delete extension-declared specs whose key is not in `keep` — what a
    /// disabled or removed extension leaves behind. Its measures are kept:
    /// dropping one would cascade away its facts, and re-enabling the
    /// extension should find its history.
    pub async fn delete_extension_specs_not_in(
        &self,
        keep: Vec<String>,
    ) -> Result<u64, DomainError> {
        self.db
            .call(move |conn| {
                if keep.is_empty() {
                    let n =
                        conn.execute("DELETE FROM metric_spec WHERE extension IS NOT NULL", [])?;
                    return Ok(n as u64);
                }
                let placeholders = vec!["?"; keep.len()].join(", ");
                let sql = format!(
                    "DELETE FROM metric_spec
                      WHERE extension IS NOT NULL AND key NOT IN ({placeholders})"
                );
                let n = conn.execute(&sql, rusqlite::params_from_iter(keep.iter()))?;
                Ok(n as u64)
            })
            .await
    }

    /// Delete extension-declared dimensions whose key is not in `keep` — a
    /// disabled or removed extension's. Extension dimensions are never
    /// promoted, so this never touches the cube's grain.
    pub async fn delete_extension_dimensions_not_in(
        &self,
        keep: Vec<String>,
    ) -> Result<u64, DomainError> {
        self.db
            .call(move |conn| {
                if keep.is_empty() {
                    let n = conn.execute(
                        "DELETE FROM dimension WHERE extension IS NOT NULL AND promoted = 0",
                        [],
                    )?;
                    return Ok(n as u64);
                }
                let placeholders = vec!["?"; keep.len()].join(", ");
                let sql = format!(
                    "DELETE FROM dimension
                      WHERE extension IS NOT NULL AND promoted = 0 AND key NOT IN ({placeholders})"
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
                let placeholders = vec!["?"; keep.len()].join(", ");
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

    /// The distinct `(producer, rule, severity, dims_json)` slices of a measure
    /// — the slice KEY only, no fact payload.
    ///
    /// This is the cheap half of [`Self::representative_facts_by_slice`]. Both
    /// scan every fact of the measure and spill to a temp b-tree (the key
    /// includes the open `dims_json` TEXT payload, so no index covers it), but
    /// this one carries 4 columns through the sorter instead of 26 and skips
    /// the rowid join-back — measured at 0.40s vs 0.78s over a 917k-fact
    /// measure. Prefer it whenever the caller's predicate reads nothing outside
    /// the slice key (`FactFilter::slice_key_only`, tsk239).
    pub async fn distinct_slice_keys(
        &self,
        measure_id: i64,
    ) -> Result<Vec<FactSliceKey>, DomainError> {
        let (generation, hit) = self.db.memo().slice_keys_get(measure_id);
        if let Some(hit) = hit {
            return Ok(hit);
        }
        let keys = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT DISTINCT c.producer, f.rule, f.severity, f.dims_json
                       FROM fact f JOIN metric_capture c ON c.id = f.capture_id
                      WHERE f.measure_id = ?1",
                )?;
                let rows = stmt.query_map(params![measure_id], |r| {
                    Ok(FactSliceKey {
                        producer: r.get(0)?,
                        rule: r.get(1)?,
                        severity: r.get(2)?,
                        dims_json: r.get(3)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        self.db
            .memo()
            .slice_keys_put(measure_id, generation, keys.clone());
        Ok(keys)
    }

    /// One representative fact per distinct `(producer, rule, severity,
    /// dims_json)` slice of a measure (tsk75) — the whole `MIN(f.id)` row of
    /// each slice. The zero-splice fallback in the effort delta only needs to
    /// learn WHICH producers emit a metric's slice; it used to load the
    /// measure's entire history to extract a handful of distinct producer
    /// names. Slice combos are bounded (rules × severities × dim payloads),
    /// never fact-count-shaped.
    ///
    /// **This is the expensive fallback — reach for [`Self::distinct_slice_keys`]
    /// first.** It exists only for predicates that read a fact column *outside*
    /// the slice key: `min_value`/`max_value` read `value`, and a `dim_eq` on
    /// `package`/`branch`/`subject`/`model` reads `path`/`subject_ref`/
    /// `subject_kind`/`branch`. Those need a real row, and "the row" is defined
    /// as the slice's lowest-id member.
    ///
    /// The query shape below looks redundant and is not. Folding the join-back
    /// away by projecting bare columns under `MIN(f.id)` (a documented SQLite
    /// extension) was measured **slower** — 1.43s vs 0.78s — because it drags
    /// all 26 columns through the group-by sorter instead of 277 rowid lookups
    /// afterwards. Don't "simplify" it back without re-measuring (tsk239).
    pub async fn representative_facts_by_slice(
        &self,
        measure_id: i64,
    ) -> Result<Vec<FactRow>, DomainError> {
        let (generation, hit) = self.db.memo().representatives_get(measure_id);
        if let Some(hit) = hit {
            return Ok(hit);
        }
        let rows = self
            .db
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
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt.query_map(params![measure_id], fact_row_mapper(conn)?)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        self.db
            .memo()
            .representatives_put(measure_id, generation, rows.clone());
        Ok(rows)
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
                let placeholders = vec!["?"; capture_ids.len()].join(", ");
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
                let rows =
                    stmt.query_map(rusqlite::params_from_iter(binds), fact_row_mapper(conn)?)?;
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
                       -- An ASSERTED capture (agent `metric.record`, synthetic writes)
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
                let rows =
                    stmt.query_map(params![measure_id, stream_id], fact_row_mapper(conn)?)?;
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
    /// The newest snapshot of `stream_id` that `producer` finished a
    /// capture for, any scan kind — where a deferred (paced) run picks up
    ///.
    pub async fn last_done_snapshot(
        &self,
        producer: &str,
        stream_id: i64,
    ) -> Result<Option<i64>, DomainError> {
        let producer = producer.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT max(c.snapshot_id) FROM metric_capture c
                       JOIN snapshot s ON s.id = c.snapshot_id
                      WHERE c.producer = ?1 AND c.status = 'done' AND s.stream_id = ?2",
                    params![producer, stream_id],
                    |r| r.get::<_, Option<i64>>(0),
                )
            })
            .await
    }

    pub async fn collector_done_for_snapshot(
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
    /// the baseline are unaffected). Runs after each successful baseline
    /// sweep. Only a producer whose facts land on a per-path measure is a
    /// baseline producer: a whole-tree restate of a complete-scope measure
    /// (`oxplow.duplicate_lines`, empty on a clean tree) is history and is
    /// never dominated (tsk709).
    ///
    /// **Invalidates that stream's cube when it drops anything** (tsk100). Deleted
    /// captures' facts cascade, and `metric_live_fact` cascades with them — but
    /// `metric_cube` rows are frozen at build time and would keep counting a fact
    /// the facts no longer have. Usually they'd agree anyway (the baseline already
    /// evicted those paths), but not for a path the sweep never restated, so we
    /// invalidate rather than reason about which prunes are safe. The cube is
    /// disposable; the next build re-folds. A prune that drops NOTHING leaves it
    /// alone — `rebuild_baseline` prunes on every boot, and wiping a healthy
    /// cube each start would turn tsk96's fix off for nothing.
    pub async fn prune_dominated_tree_captures(&self, stream_id: i64) -> Result<u64, DomainError> {
        let n = self
            .db
            .transaction(move |tx| {
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
                           -- A baseline producer: its facts land on a per-path
                           -- measure somewhere. A whole-tree restate of a
                           -- complete-scope measure (empty on a clean tree)
                           -- has no fact at all and is history, not a baseline.
                           AND EXISTS (
                             SELECT 1 FROM metric_capture pc
                               JOIN fact pf ON pf.capture_id = pc.id
                               JOIN measure pm ON pm.id = pf.measure_id
                              WHERE pc.stream_id = lf.stream_id
                                AND pc.producer = lf.producer
                                AND pm.capture_scope = 'per-path'
                           )
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
                Ok(n as u64)
            })
            .await?;
        // Deleted captures took their facts with them: forget what was
        // memoized about any measure.
        if n > 0 {
            self.db.memo().invalidate_all();
        }
        Ok(n)
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
            .transaction(move |tx| {
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
        let n = self
            .db
            .transaction(move |tx| {
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
                Ok(n as u64)
            })
            .await?;
        // Deleted captures took their facts with them: forget what was
        // memoized about any measure.
        if n > 0 {
            self.db.memo().invalidate_all();
        }
        Ok(n)
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
                let rows =
                    stmt.query_map(params![measure_id, stream_id], fact_row_mapper(conn)?)?;
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
                let placeholders = vec!["?"; capture_ids.len()].join(", ");
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

    /// tsk978: a store write runs in the retried write transaction — the
    /// measure seeding that failed a fresh daemon's boot with "database is
    /// locked" lands while another writer holds the lock past the pool's
    /// busy wait, then lets go.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_store_write_outlasts_another_writers_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("facts.sqlite");
        let store = SqliteFactStore::new(Database::open(&path).unwrap());
        let (held, holding) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            held.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(6_500));
            conn.execute_batch("COMMIT").unwrap();
        });
        holding.recv().unwrap();
        store
            .upsert_measure(NewMeasure::new("acme.todo", "TODOs"))
            .await
            .expect("retried once the lock was free");
        holder.join().unwrap();
    }

    /// stream(1) + thread(1) + task + effort so capture FKs resolve and the
    /// effort-GC test has a real effort to delete.
    #[tokio::test]
    async fn extension_scoped_specs_and_measures_round_trip_and_prune() {
        let store = fixture().await;
        let mut m = NewMeasure::new("acme.todo", "TODOs");
        m.scope = "extension:acme".into();
        store.upsert_measure(m).await.unwrap();
        assert_eq!(
            store.get_measure("acme.todo").await.unwrap().unwrap().scope,
            "extension:acme"
        );
        for key in ["acme.todos", "acme.gone"] {
            let mut s = NewMetricSpec::base(key, key, "acme.todo", "sum");
            s.scope = "extension:acme".into();
            store.upsert_spec(s).await.unwrap();
        }
        let mut p = NewMetricSpec::base("proj.x", "X", "acme.todo", "sum");
        p.scope = "project".into();
        store.upsert_spec(p).await.unwrap();
        assert_eq!(
            store.get_spec("acme.todos").await.unwrap().unwrap().scope,
            "extension:acme"
        );

        // Only extension specs that are no longer declared go; project ones
        // (and measures, which would take their facts with them) stay.
        let n = store
            .delete_extension_specs_not_in(vec!["acme.todos".into()])
            .await
            .unwrap();
        assert_eq!(n, 1);
        let keys: Vec<String> = store
            .list_specs()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(keys.contains(&"acme.todos".to_string()) && keys.contains(&"proj.x".to_string()));
        assert!(!keys.contains(&"acme.gone".to_string()));
        assert!(store.get_measure("acme.todo").await.unwrap().is_some());
    }

    /// tsk945: a dimension key has one name — its namespaced one. A fact
    /// carrying a bare key is refused, naming the conformed key when the
    /// catalog has one; nothing of the capture is written.
    #[tokio::test]
    async fn a_bare_dimension_key_is_refused_naming_the_conformed_one() {
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure::new("acme.lines", "Lines"))
            .await
            .unwrap();
        let with_dims = |dims: &str| NewFact {
            dims_json: Some(dims.into()),
            ..NewFact::new(m, 1.0)
        };
        let err = store
            .record_facts(
                NewMetricCapture::done(1, "acme", "gauge"),
                vec![with_dims(r#"{"language":"rust"}"#)],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DomainError::Invalid(m) if m.contains("`language`") && m.contains("`oxplow.language`")),
            "{err:?}"
        );
        let err = store
            .record_facts(
                NewMetricCapture::done(1, "acme", "gauge"),
                vec![with_dims(r#"{"zone":"api"}"#)],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DomainError::Invalid(m) if m.contains("`zone`") && m.contains("namespace")),
            "{err:?}"
        );
        assert!(store
            .captures_for_producers(vec!["acme".into()])
            .await
            .unwrap()
            .is_empty());
        store
            .record_facts(
                NewMetricCapture::done(1, "acme", "gauge"),
                vec![with_dims(r#"{"oxplow.language":"rust","acme.zone":"api"}"#)],
            )
            .await
            .unwrap();
    }

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
                    "INSERT INTO effort (id, work_item, thread_id, started_at, ended_at)
                     VALUES (1, 'work_item:oxplow:tsk' || ?1, 1, '2026-06-30T10:00:00.000000Z', '2026-06-30T11:00:00.000000Z')",
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
    /// `(path, value)`. Mirrors what `record_collector_facts` writes.
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

    /// P4.11 (tsk496): the `v_tree_fact` model is the engine's fold — for
    /// measure `m` (made per-path, which the model reads) on stream 1, the
    /// same facts `latest_tree_facts` returns.
    async fn model_agrees(store: &SqliteFactStore, m: i64) {
        let mut engine: Vec<i64> = store
            .latest_tree_facts(m, Some(1))
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect();
        engine.sort();
        let model: Vec<i64> = store
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE measure SET capture_scope = 'per-path' WHERE id = ?1",
                    [m],
                )?;
                let mut st = conn.prepare(
                    "SELECT t.id FROM v_tree_fact t JOIN measure m ON m.key = t.measure_key
                     WHERE m.id = ?1 AND t.stream_id = 1 ORDER BY t.id",
                )?;
                let ids = st
                    .query_map([m], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<i64>>>()?;
                Ok(ids)
            })
            .await
            .unwrap();
        assert_eq!(
            model, engine,
            "v_tree_fact disagrees with the engine's fold"
        );
    }

    /// P4.11 (tsk496): `v_function` is each function's latest complexity —
    /// a rescan of its file replaces it, a function removed from the file
    /// goes, and a file not rescanned keeps its functions.
    #[tokio::test]
    async fn v_function_is_the_latest_complexity_per_function() {
        let store = fixture().await;
        let m = store
            .db
            .call(|conn| {
                conn.query_row(
                    "SELECT id FROM measure WHERE key = 'oxplow.complexity'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .unwrap();
        let capture = |snap: i64, at_: &str, fns: &[(&str, &str, f64)]| {
            let mut c = NewMetricCapture::done(1, "complexity", "metric:complexity");
            c.snapshot_id = Some(snap);
            c.captured_at = Some(at(at_));
            let facts: Vec<NewFact> = fns
                .iter()
                .map(|(path, name, v)| NewFact {
                    subject_kind: Some("symbol".into()),
                    subject_ref: Some(format!("symbol:{path}::{name}")),
                    path: Some((*path).to_string()),
                    line: Some(3),
                    dims_json: Some(r#"{"oxplow.language":"rust"}"#.into()),
                    ..NewFact::new(m, *v)
                })
                .collect();
            (c, facts)
        };
        snapshot_with(&store, 1, &[("a.rs", "oxplow"), ("b.rs", "oxplow")]).await;
        let (c, f) = capture(
            1,
            "2026-06-30T10:00:00.000000Z",
            &[
                ("a.rs", "one", 4.0),
                ("a.rs", "two", 7.0),
                ("b.rs", "three", 2.0),
            ],
        );
        store.record_facts(c, f).await.unwrap();
        snapshot_with(&store, 2, &[("a.rs", "oxplow")]).await;
        let (c, f) = capture(2, "2026-06-30T11:00:00.000000Z", &[("a.rs", "one", 9.0)]);
        store.record_facts(c, f).await.unwrap();
        let rows: Vec<(String, String, f64, Option<String>)> = store
            .db
            .call(|conn| {
                let mut st = conn.prepare(
                    "SELECT name, path, complexity, language FROM v_function
                     WHERE stream_id = 1 ORDER BY name",
                )?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("one".into(), "a.rs".into(), 9.0, Some("rust".into())),
                ("three".into(), "b.rs".into(), 2.0, Some("rust".into())),
            ]
        );
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
        model_agrees(&store, m).await;
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
        model_agrees(&store, m).await;
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
        model_agrees(&store, m).await;
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
        model_agrees(&store, m).await;
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
        model_agrees(&store, m).await;
    }

    #[tokio::test]
    async fn asserted_capture_with_snapshot_restates_only_its_emitted_paths() {
        // tsk72 direction: `metric.record` captures now carry a snapshot for
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
        model_agrees(&store, m).await;
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
    async fn an_empty_whole_tree_restate_is_not_a_baseline_that_prunes() {
        // P7 review (tsk709): a whole-tree collector on a complete-scope
        // measure (`oxplow.duplicate_lines`) restates the tree on every ref
        // move, and a clean tree's restate is an EMPTY full capture. Empty
        // captures carry no fact at all, so the "carries a non-per-path fact"
        // guard can't tell them from a per-path baseline — and the previous
        // empty restate was pruned on every commit, wiping the stream's cube
        // each time. The prune is about per-path baselines: a producer with
        // no per-path fact anywhere is not one.
        let store = fixture().await;
        let complete = store
            .upsert_measure(NewMeasure::new("acme.dups", "acme.dups"))
            .await
            .unwrap();
        let per_path = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-path".into(),
                ..NewMeasure::new("acme.hits", "acme.hits")
            })
            .await
            .unwrap();
        snapshot_with(&store, 1, &[("a.rs", "oxplow")]).await;
        let first = full_capture(
            &store,
            "dup",
            1,
            "2026-06-30T09:00:00.000000Z",
            complete,
            &[],
        )
        .await;
        let second = full_capture(
            &store,
            "dup",
            1,
            "2026-06-30T10:00:00.000000Z",
            complete,
            &[],
        )
        .await;
        // A real per-path baseline beside it, so the cube has something built.
        let hits = full_capture(
            &store,
            "g",
            1,
            "2026-06-30T09:30:00.000000Z",
            per_path,
            &[("a.rs", 8.0)],
        )
        .await;
        store
            .write_cube_rows(
                per_path,
                1,
                None,
                hits,
                at("2026-06-30T09:30:00.000000Z"),
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
        assert_eq!(
            pruned, 0,
            "an empty restate of a complete measure dominates nothing"
        );
        let alive: Vec<i64> = store
            .captures_for_producers(vec!["dup".into()])
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(alive, vec![first, second], "the metric's history stands");
        assert!(
            store.cube_watermark(per_path, 1).await.unwrap().is_some(),
            "the stream's cube survives a clean tree's restate"
        );
    }

    /// tsk704: a per-subject partition's seed is the latest visible capture
    /// per (producer, subject) — every fact of that capture under that key —
    /// computed in SQL over the visible captures, never by loading history.
    #[tokio::test]
    async fn the_per_subject_seed_is_the_latest_visible_capture_per_subject() {
        let store = fixture().await;
        let m = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.test_case", "acme.test_case")
            })
            .await
            .unwrap();
        let subject = |s: &str, v: f64| NewFact {
            subject_ref: Some(s.into()),
            ..NewFact::new(m, v)
        };
        let at = |t: &str| at(t);
        let capture = |producer: &str, when: &str| NewMetricCapture {
            captured_at: Some(at(when)),
            ..NewMetricCapture::done(1, producer, "t")
        };
        // c1: A, B, a scalar (no subject, no path) and a path-only fact.
        let c1 = store
            .record_facts(
                capture("tests", "2026-06-30T08:00:00.000000Z"),
                vec![
                    subject("A", 1.0),
                    subject("B", 1.5),
                    NewFact::new(m, 7.0),
                    NewFact {
                        path: Some("src/p.rs".into()),
                        ..NewFact::new(m, 3.0)
                    },
                ],
            )
            .await
            .unwrap();
        // c2 restates A (twice: two facts under one key both stay).
        let c2 = store
            .record_facts(
                capture("tests", "2026-06-30T09:00:00.000000Z"),
                vec![subject("A", 2.0), subject("A", 2.5)],
            )
            .await
            .unwrap();
        // Another producer's A is its own key.
        let c3 = store
            .record_facts(
                capture("other", "2026-06-30T09:30:00.000000Z"),
                vec![subject("A", 9.0)],
            )
            .await
            .unwrap();
        // c4 is not visible to the seeded branch: ignored.
        let c4 = store
            .record_facts(
                capture("tests", "2026-06-30T10:00:00.000000Z"),
                vec![subject("A", 100.0)],
            )
            .await
            .unwrap();
        let facts_of = |id: i64| {
            let store = &store;
            async move {
                let mut v: Vec<(f64, i64)> = store
                    .facts_for_captures(m, vec![id])
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|f| (f.value, f.id))
                    .collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v
            }
        };
        let (f1, f2, f3) = (facts_of(c1).await, facts_of(c2).await, facts_of(c3).await);
        let id_of =
            |rows: &[(f64, i64)], value: f64| rows.iter().find(|(v, _)| *v == value).unwrap().1;

        let mut seed = store
            .live_seed_per_subject(m, vec![c1, c2, c3], "\u{0}repo-scalar")
            .await
            .unwrap();
        seed.sort();
        let mut want = vec![
            ("other".to_string(), "A".to_string(), id_of(&f3, 9.0)),
            ("tests".to_string(), "A".to_string(), id_of(&f2, 2.0)),
            ("tests".to_string(), "A".to_string(), id_of(&f2, 2.5)),
            ("tests".to_string(), "B".to_string(), id_of(&f1, 1.5)),
            ("tests".to_string(), "src/p.rs".to_string(), id_of(&f1, 3.0)),
            (
                "tests".to_string(),
                "\u{0}repo-scalar".to_string(),
                id_of(&f1, 7.0),
            ),
        ];
        want.sort();
        assert_eq!(seed, want);
        let _ = c4;
    }

    /// tsk733: a test run records per-case facts only where they say
    /// something new — every failure, and a pass or skip only when the test
    /// is new on the branch, changed status, or moved its duration past the
    /// threshold — and keeps a per-test summary row up to date in the same
    /// transaction.
    #[tokio::test]
    async fn a_test_run_records_what_changed_and_keeps_the_per_test_summary() {
        let store = fixture().await;
        let cases = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.test_case", "acme.test_case")
            })
            .await
            .unwrap();
        let durations = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.test_duration", "acme.test_duration")
            })
            .await
            .unwrap();
        let result = |t: &str, status: &str, ms: f64| TestCaseResult {
            subject: format!("test:{t}"),
            status: status.into(),
            time_ms: Some(ms),
            dims_json: Some(format!(r#"{{"oxplow.status":"{status}"}}"#)),
        };
        let run = |branch: &str, when: &str| NewMetricCapture {
            captured_at: Some(at(when)),
            branch: Some(branch.into()),
            ..NewMetricCapture::done(1, "tests", "t")
        };
        let recorded = |id: i64| {
            let store = &store;
            async move {
                let mut status: Vec<String> = store
                    .facts_for_captures(cases, vec![id])
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|f| f.subject_ref.unwrap())
                    .collect();
                status.sort();
                let mut timed: Vec<String> = store
                    .facts_for_captures(durations, vec![id])
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|f| f.subject_ref.unwrap())
                    .collect();
                timed.sort();
                (status, timed)
            }
        };
        let sets = |a: &[&str]| a.iter().map(|t| format!("test:{t}")).collect::<Vec<_>>();

        // First run on main: everything is new.
        let r1 = store
            .record_test_run(
                run("main", "2026-06-01T08:00:00.000000Z"),
                vec![
                    result("a", "passed", 100.0),
                    result("b", "failed", 10.0),
                    result("c", "skipped", 0.0),
                    result("tiny", "passed", 5.0),
                ],
                cases,
                Some(durations),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            recorded(r1).await,
            (
                sets(&["a", "b", "c", "tiny"]),
                sets(&["a", "b", "c", "tiny"])
            )
        );

        // The same results again, a little slower: only the failure.
        let r2 = store
            .record_test_run(
                run("main", "2026-06-01T09:00:00.000000Z"),
                vec![
                    result("a", "passed", 140.0),
                    result("b", "failed", 10.0),
                    result("c", "skipped", 0.0),
                    result("tiny", "passed", 12.0),
                ],
                cases,
                Some(durations),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            recorded(r2).await,
            (sets(&["b"]), sets(&[])),
            "40% slower and a 7 ms move are both inside the tolerance"
        );

        // a drifts past 50% of its LAST RECORDED duration; b goes green.
        let r3 = store
            .record_test_run(
                run("main", "2026-06-01T10:00:00.000000Z"),
                vec![
                    result("a", "passed", 160.0),
                    result("b", "passed", 10.0),
                    result("c", "skipped", 0.0),
                    result("tiny", "passed", 5.0),
                ],
                cases,
                Some(durations),
                None,
            )
            .await
            .unwrap();
        assert_eq!(recorded(r3).await, (sets(&["b"]), sets(&["a"])));

        // Another branch starts from nothing of its own.
        let r4 = store
            .record_test_run(
                run("feat", "2026-06-01T11:00:00.000000Z"),
                vec![result("a", "passed", 100.0)],
                cases,
                Some(durations),
                None,
            )
            .await
            .unwrap();
        assert_eq!(recorded(r4).await, (sets(&["a"]), sets(&["a"])));

        let stats = store
            .test_case_stats(1, Some("main".into()), "tests")
            .await
            .unwrap();
        let b = stats.iter().find(|s| s.subject == "test:b").unwrap();
        assert_eq!(
            (b.runs, b.failures, b.flips, b.last_status.as_str()),
            (3, 2, 1, "passed")
        );
        assert_eq!(
            b.last_failed_at.as_deref(),
            Some("2026-06-01T09:00:00.000000Z")
        );
        assert_eq!(
            b.last_passed_at.as_deref(),
            Some("2026-06-01T10:00:00.000000Z")
        );
        let a = stats.iter().find(|s| s.subject == "test:a").unwrap();
        assert_eq!(
            (a.last_ms, a.max_ms, a.recorded_ms),
            (Some(160.0), Some(160.0), Some(160.0))
        );
        assert_eq!(a.mean_ms, Some((100.0 + 140.0 + 160.0) / 3.0));
        assert_eq!(a.last_run_id, Some(r3));
        assert_eq!(stats.len(), 4);
    }

    /// A run replayed under the same idempotency key records nothing and
    /// counts nothing twice.
    #[tokio::test]
    async fn a_replayed_test_run_changes_nothing() {
        let store = fixture().await;
        let cases = store
            .upsert_measure(NewMeasure {
                capture_scope: "per-subject".into(),
                ..NewMeasure::new("acme.test_case", "acme.test_case")
            })
            .await
            .unwrap();
        let run = || NewMetricCapture {
            idempotency_key: Some("test-run:evt1".into()),
            branch: Some("main".into()),
            ..NewMetricCapture::done(1, "tests", "t")
        };
        let case = || TestCaseResult {
            subject: "test:a".into(),
            status: "failed".into(),
            time_ms: None,
            dims_json: None,
        };
        let first = store
            .record_test_run(run(), vec![case()], cases, None, None)
            .await
            .unwrap();
        let again = store
            .record_test_run(run(), vec![case()], cases, None, None)
            .await
            .unwrap();
        assert_eq!(first, again);
        let stats = store
            .test_case_stats(1, Some("main".into()), "tests")
            .await
            .unwrap();
        assert_eq!((stats[0].runs, stats[0].failures), (1, 1));
    }

    #[tokio::test]
    async fn a_prune_that_drops_nothing_leaves_the_cube_alone() {
        // The other half, and NOT hypothetical: `rebuild_baseline` prunes on
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

    /// tsk216: the WAL parks at the high-water mark of the biggest write burst
    /// and never shrinks on its own, so the daily pass truncates it.
    #[tokio::test]
    async fn checkpoint_wal_truncates_the_write_ahead_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let db = Database::open(&path).unwrap();
        // A capture FKs to `streams`, and this test needs a FILE-backed DB (an
        // in-memory one has no WAL), so seed the row `fixture()` would have.
        db.call(|conn| {
            conn.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, created_at, updated_at)
                 VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                ["2026-06-30T00:00:00Z"],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let store = SqliteFactStore::new(db);
        let m = store
            .upsert_measure(NewMeasure::new("acme.m", "M"))
            .await
            .unwrap();
        // Write enough to grow the WAL past its automatic-checkpoint threshold.
        for _ in 0..40 {
            store
                .record_facts(
                    NewMetricCapture::done(1, "p", "s"),
                    (0..200).map(|i| NewFact::new(m, i as f64)).collect(),
                )
                .await
                .unwrap();
        }
        let wal = path.with_extension("sqlite-wal");
        let before = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);

        store.checkpoint_wal().await.unwrap();

        let after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(
            after < before || before == 0,
            "checkpoint must return the WAL's space: {before} -> {after}",
        );
        // And the data is still readable — a checkpoint moves frames into the
        // main DB, it never drops them.
        assert_eq!(store.facts_for_measure(m).await.unwrap().len(), 40 * 200);
    }

    /// tsk211: compaction must bound `detail_json` growth WITHOUT touching a
    /// single metric value — that property is what lets it default to ON.
    #[tokio::test]
    async fn compacting_details_keeps_the_newest_per_producer_and_never_drops_a_fact() {
        let store = fixture().await;
        let m = measure(&store, "acme.m").await;
        let mut ids = Vec::new();
        // Two producers, 4 captures each, oldest → newest.
        for h in 0..4 {
            for p in ["tests", "coverage"] {
                let mut cap = NewMetricCapture::done(1, p, "s");
                cap.captured_at = Some(at(&format!("2026-06-{:02}T10:00:00.000000Z", 10 + h)));
                cap.detail_json = Some(format!(r#"{{"kind":"{p}-detail","h":{h}}}"#));
                ids.push((
                    p,
                    h,
                    store
                        .record_facts(cap, vec![NewFact::new(m, 1.0)])
                        .await
                        .unwrap(),
                ));
            }
        }
        let facts_before = store.facts_for_measure(m).await.unwrap().len();

        // Keep the newest 2 per producer: 2 of each producer's 4 are compacted.
        let n = store.compact_capture_details(None, Some(2)).await.unwrap();
        assert_eq!(n, 4, "2 stale captures for each of the 2 producers");

        for (p, h, id) in &ids {
            let cap = store.get_capture(*id).await.unwrap().unwrap();
            let kept = *h >= 2; // newest two hours per producer
            assert_eq!(
                cap.detail_json.is_some(),
                kept,
                "producer {p} hour {h}: detail retention",
            );
            // The capture itself always survives — this is compaction.
            assert_eq!(cap.producer, *p);
        }
        assert_eq!(
            store.facts_for_measure(m).await.unwrap().len(),
            facts_before,
            "compaction must never delete a fact — no metric value may change",
        );

        // Re-running is a no-op (nothing left to compact at this bound).
        assert_eq!(
            store.compact_capture_details(None, Some(2)).await.unwrap(),
            0
        );

        // The age bound reaches the rest.
        let n = store
            .compact_capture_details(Some(at("2026-06-14T00:00:00.000000Z")), None)
            .await
            .unwrap();
        assert_eq!(n, 4, "the remaining 4 are all older than the cutoff");
        assert_eq!(
            store.facts_for_measure(m).await.unwrap().len(),
            facts_before,
            "still no fact lost",
        );

        // Both bounds disabled = explicit no-op, not "compact everything".
        assert_eq!(store.compact_capture_details(None, None).await.unwrap(), 0);
    }

    /// tsk196. The cube's read cache keys on `cube_version`, so the version
    /// must move on EVERY mutation of `metric_cube` — not just the wipes
    /// `cube_epoch` fences. A fold that left the version parked would let the
    /// cache serve pre-fold rows forever, i.e. metrics that silently stop
    /// updating: strictly worse than the CPU cost the cache exists to avoid.
    #[tokio::test]
    async fn cube_version_advances_on_a_fold_even_though_the_epoch_does_not() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.tokens").await;
        let c = store
            .record_facts(
                NewMetricCapture::done(1, "otel-tokens", "otel"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();

        let epoch_before = store.cube_epoch().await.unwrap();
        let version_before = store.cube_version().await.unwrap();

        store
            .write_cube_rows(
                m,
                1,
                None,
                c,
                at("2026-06-30T10:00:00.000000Z"),
                vec![NewCubeRow {
                    producer: "otel-tokens".into(),
                    dims_key: "{}".into(),
                    fact_count: 1,
                    value_sum: 1.0,
                    value_min: Some(1.0),
                    value_max: Some(1.0),
                    numerator: 0.0,
                    denominator: 0.0,
                }],
                epoch_before,
            )
            .await
            .unwrap();

        assert!(
            store.cube_version().await.unwrap() > version_before,
            "the fold must advance the cache's invalidation token"
        );
        assert_eq!(
            store.cube_epoch().await.unwrap(),
            epoch_before,
            "and must NOT advance the epoch — that would abort concurrent folds"
        );
    }

    /// tsk196. `metric_cube.capture_id` is `ON DELETE CASCADE`, so deleting a
    /// capture (archiving a stream, a prune) removes cube rows with NO Rust
    /// call site involved. The version is maintained by a trigger precisely so
    /// a path nobody remembered still invalidates the cache.
    #[tokio::test]
    async fn cube_version_advances_when_a_cascade_deletes_cube_rows() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.tokens").await;
        let c = store
            .record_facts(
                NewMetricCapture::done(1, "otel-tokens", "otel"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        store
            .write_cube_rows(
                m,
                1,
                None,
                c,
                at("2026-06-30T10:00:00.000000Z"),
                vec![NewCubeRow {
                    producer: "otel-tokens".into(),
                    dims_key: "{}".into(),
                    fact_count: 1,
                    value_sum: 1.0,
                    value_min: Some(1.0),
                    value_max: Some(1.0),
                    numerator: 0.0,
                    denominator: 0.0,
                }],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();

        let version_before = store.cube_version().await.unwrap();
        let db = store.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute("DELETE FROM metric_capture WHERE id = ?1", params![c])?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();

        assert!(
            store.cube_version().await.unwrap() > version_before,
            "a cascade delete must still invalidate the cache"
        );
    }

    /// tsk196. The point of the cache: repeated identical reads inside one
    /// version agree, but a fold is visible on the very next read. Staleness
    /// is not a permitted trade here — that's why this keys on a version
    /// rather than a TTL.
    #[tokio::test]
    async fn cube_rows_are_cached_within_a_version_and_refresh_after_a_fold() {
        let store = fixture().await;
        let m = measure(&store, "oxplow.tokens").await;
        let row = |v: f64| NewCubeRow {
            producer: "otel-tokens".into(),
            dims_key: "{}".into(),
            fact_count: 1,
            value_sum: v,
            value_min: Some(v),
            value_max: Some(v),
            numerator: 0.0,
            denominator: 0.0,
        };
        let c1 = store
            .record_facts(
                NewMetricCapture::done(1, "otel-tokens", "otel"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        store
            .write_cube_rows(
                m,
                1,
                None,
                c1,
                at("2026-06-30T10:00:00.000000Z"),
                vec![row(1.0)],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();

        let first = store.cube_rows_for_measure(m, Some(1)).await.unwrap();
        let cached = store.cube_rows_for_measure(m, Some(1)).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(cached.len(), 1, "a repeat read inside the version agrees");

        // A second capture folds in — the next read must see it.
        let c2 = store
            .record_facts(
                NewMetricCapture::done(1, "otel-tokens", "otel"),
                vec![NewFact::new(m, 2.0)],
            )
            .await
            .unwrap();
        store
            .write_cube_rows(
                m,
                1,
                None,
                c2,
                at("2026-06-30T11:00:00.000000Z"),
                vec![row(2.0)],
                store.cube_epoch().await.unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            store.cube_rows_for_measure(m, Some(1)).await.unwrap().len(),
            2,
            "the fold must be visible on the next read, not after a TTL"
        );
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
        // V45 + turn/effort/nudge in V46 + oxplow.effort_test_outcome in V53 +
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
                    closest_vcs_rev: Some("abc1234".into()),
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
            .all(|f| f.closest_vcs_rev.as_deref() == Some("abc1234")));
        // Oldest-first within the capture is by fact id (insertion order).
        assert_eq!(rows[0].subject_ref.as_deref(), Some("src/a.rs::foo"));
        assert_eq!(rows[0].value, 14.0);
        assert_eq!(rows[1].value, 3.0);
    }

    #[tokio::test]
    async fn producers_for_measure_memo_sees_a_write_through_another_store() {
        // tsk130: `producers_for_measure` is memoized (it was ~46% of backend
        // CPU). The memo lives on `Database`, not on the store, because the app
        // builds SEVERAL stores over one Database — so a write through any of
        // them has to invalidate what the others memoized. This is the test for
        // that: a per-store cache passes the first two asserts and fails the
        // last, silently hiding a producer from every read until the next write.
        let store = fixture().await;
        let m = measure(&store, "acme.hits").await;

        store
            .record_facts(
                NewMetricCapture::done(1, "alpha", "builtin"),
                vec![NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        assert_eq!(store.producers_for_measure(m).await.unwrap(), vec!["alpha"]);
        // Served from the memo the second time — same answer.
        assert_eq!(store.producers_for_measure(m).await.unwrap(), vec!["alpha"]);

        let other = SqliteFactStore::new(store.db.clone());
        other
            .record_facts(
                NewMetricCapture::done(1, "beta", "builtin"),
                vec![NewFact::new(m, 2.0)],
            )
            .await
            .unwrap();

        let mut got = store.producers_for_measure(m).await.unwrap();
        got.sort();
        assert_eq!(
            got,
            vec!["alpha", "beta"],
            "a fact write through another store over the same Database must \
             invalidate the memo"
        );
    }

    /// A fact write forgets only the memo of the measures it wrote: a token
    /// fact landing every few seconds mustn't keep every other measure's
    /// producer list cold.
    #[tokio::test]
    async fn a_fact_write_forgets_only_its_own_measures_producers() {
        let store = fixture().await;
        let a = measure(&store, "acme.a").await;
        let b = measure(&store, "acme.b").await;
        for m in [a, b] {
            store
                .record_facts(
                    NewMetricCapture::done(1, "alpha", "builtin"),
                    vec![NewFact::new(m, 1.0)],
                )
                .await
                .unwrap();
            store.producers_for_measure(m).await.unwrap();
        }
        let memoized = |m: i64| store.db.memo().producers_get(m).1.is_some();
        assert!(memoized(a) && memoized(b));
        store
            .record_facts(
                NewMetricCapture::done(1, "beta", "builtin"),
                vec![NewFact::new(b, 2.0)],
            )
            .await
            .unwrap();
        assert!(memoized(a), "another measure's write leaves it");
        assert!(!memoized(b), "its own write forgets it");
        let mut got = store.producers_for_measure(b).await.unwrap();
        got.sort();
        assert_eq!(got, vec!["alpha", "beta"]);
    }

    /// A measure's slice keys and representative rows are memoized like its
    /// producers: kept across other measures' writes, forgotten by its own,
    /// and by a prune that deleted facts.
    #[tokio::test]
    async fn slice_reads_are_memoized_per_measure() {
        let store = fixture().await;
        let a = measure(&store, "acme.a").await;
        let b = measure(&store, "acme.b").await;
        for m in [a, b] {
            store
                .record_facts(
                    NewMetricCapture::done(1, "alpha", "builtin"),
                    vec![NewFact::new(m, 1.0)],
                )
                .await
                .unwrap();
        }
        store.distinct_slice_keys(a).await.unwrap();
        store.representative_facts_by_slice(a).await.unwrap();
        let memoized = |m: i64| {
            let memo = store.db.memo();
            (
                memo.slice_keys_get(m).1.is_some(),
                memo.representatives_get(m).1.is_some(),
            )
        };
        assert_eq!(memoized(a), (true, true));
        store
            .record_facts(
                NewMetricCapture::done(1, "beta", "builtin"),
                vec![NewFact::new(b, 2.0)],
            )
            .await
            .unwrap();
        assert_eq!(memoized(a), (true, true), "another measure's write");
        store
            .record_facts(
                NewMetricCapture::done(1, "beta", "builtin"),
                vec![NewFact::new(a, 2.0)],
            )
            .await
            .unwrap();
        assert_eq!(memoized(a), (false, false), "its own write");
        assert_eq!(store.distinct_slice_keys(a).await.unwrap().len(), 2);
        store.db.memo().invalidate_all();
        assert_eq!(memoized(a), (false, false), "a deletion forgets everything");
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
            .call(|conn| conn.execute("DELETE FROM effort WHERE id = 1", []))
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
    async fn representative_facts_by_slice_returns_the_lowest_id_row_of_each_slice() {
        // tsk239/tsk242: the rows this returns are FULL facts, and the caller
        // runs a `FactFilter` over them — a filter that can read `value`,
        // `path`, `subject_ref`, `subject_kind` and `branch`, none of which are
        // part of the (producer, rule, severity, dims_json) slice key. So the
        // contract is not just "one row per slice": it is "the MIN(id) row of
        // each slice, whole". Collapsing this to a DISTINCT over the slice
        // tuple would keep this test's LENGTH assert green and silently change
        // which producers get zero-filled.
        let store = fixture().await;
        let m = measure(&store, "oxplow.lint_hit").await;
        let hit = |sev: &str, rule: &str, value: f64, path: &str| NewFact {
            severity: Some(sev.into()),
            rule: Some(rule.into()),
            path: Some(path.into()),
            ..NewFact::new(m, value)
        };
        // Two producers; `alpha` emits the same slice three times with
        // different values/paths, so the representative is unambiguous.
        store
            .record_facts(
                NewMetricCapture::done(1, "alpha", "analysis"),
                vec![
                    hit("error", "E1", 1.0, "src/first.rs"),
                    hit("error", "E1", 9.0, "src/second.rs"),
                    hit("warning", "W1", 2.0, "src/third.rs"),
                ],
            )
            .await
            .unwrap();
        store
            .record_facts(
                NewMetricCapture::done(1, "alpha", "analysis"),
                vec![hit("error", "E1", 7.0, "src/fourth.rs")],
            )
            .await
            .unwrap();
        store
            .record_facts(
                NewMetricCapture::done(1, "beta", "analysis"),
                vec![hit("error", "E1", 5.0, "src/fifth.rs")],
            )
            .await
            .unwrap();

        let reps = store.representative_facts_by_slice(m).await.unwrap();
        // Three slices: (alpha,E1,error), (alpha,W1,warning), (beta,E1,error).
        assert_eq!(reps.len(), 3, "one representative per distinct slice");
        let mut got: Vec<(String, String, f64, String)> = reps
            .iter()
            .map(|f| {
                (
                    f.producer.clone(),
                    f.rule.clone().unwrap(),
                    f.value,
                    f.path.clone().unwrap(),
                )
            })
            .collect();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            got,
            vec![
                // The FIRST alpha/E1/error fact — not the 9.0 one beside it in
                // the same capture, nor the 7.0 one in the later capture.
                (
                    "alpha".to_string(),
                    "E1".to_string(),
                    1.0,
                    "src/first.rs".to_string()
                ),
                (
                    "alpha".to_string(),
                    "W1".to_string(),
                    2.0,
                    "src/third.rs".to_string()
                ),
                (
                    "beta".to_string(),
                    "E1".to_string(),
                    5.0,
                    "src/fifth.rs".to_string()
                ),
            ],
            "each row is the whole MIN(id) fact of its slice"
        );
    }

    #[tokio::test]
    async fn distinct_slice_keys_matches_the_representative_scans_slices() {
        // tsk239/tsk242: the cheap producer-discovery path. It must enumerate
        // EXACTLY the slices the expensive one does — it's chosen at runtime as
        // a drop-in for it whenever the caller's predicate reads only the slice
        // key, so any divergence is a silent change in which producers zero-fill.
        let store = fixture().await;
        let m = measure(&store, "oxplow.test_case").await;
        let case = |status: &str, rule: Option<&str>, value: f64| NewFact {
            dims_json: Some(format!(r#"{{"oxplow.status":"{status}"}}"#)),
            rule: rule.map(str::to_string),
            ..NewFact::new(m, value)
        };
        store
            .record_facts(
                NewMetricCapture::done(1, "tests", "junit"),
                vec![
                    case("passed", None, 1.0),
                    case("passed", None, 2.0),
                    case("failed", None, 3.0),
                    case("failed", Some("flaky"), 4.0),
                ],
            )
            .await
            .unwrap();
        store
            .record_facts(
                NewMetricCapture::done(1, "e2e", "junit"),
                vec![case("passed", None, 5.0)],
            )
            .await
            .unwrap();

        let mut keys = store.distinct_slice_keys(m).await.unwrap();
        let mut from_reps: Vec<FactSliceKey> = store
            .representative_facts_by_slice(m)
            .await
            .unwrap()
            .into_iter()
            .map(|f| FactSliceKey {
                producer: f.producer,
                rule: f.rule,
                severity: f.severity,
                dims_json: f.dims_json,
            })
            .collect();
        let sort = |v: &mut Vec<FactSliceKey>| {
            v.sort_by(|a, b| {
                (&a.producer, &a.rule, &a.severity, &a.dims_json).cmp(&(
                    &b.producer,
                    &b.rule,
                    &b.severity,
                    &b.dims_json,
                ))
            })
        };
        sort(&mut keys);
        sort(&mut from_reps);
        assert_eq!(
            keys.len(),
            4,
            "tests×{{passed,failed,failed+flaky}} + e2e×passed"
        );
        assert_eq!(keys, from_reps, "the two scans must enumerate one set");
    }

    #[tokio::test]
    async fn representative_facts_by_slice_separates_slices_by_dims_json() {
        // The slice key includes the open `dims_json` payload, so two facts
        // that agree on producer/rule/severity but carry different dims are
        // different slices (this is why the GROUP BY can't ride an index).
        let store = fixture().await;
        let m = measure(&store, "oxplow.complexity").await;
        let with_dims = |dims: &str, value: f64| NewFact {
            dims_json: Some(dims.into()),
            ..NewFact::new(m, value)
        };
        store
            .record_facts(
                NewMetricCapture::done(1, "metrics", "builtin"),
                vec![
                    with_dims(r#"{"oxplow.language":"rust"}"#, 1.0),
                    with_dims(r#"{"oxplow.language":"rust"}"#, 2.0),
                    with_dims(r#"{"oxplow.language":"ts"}"#, 3.0),
                ],
            )
            .await
            .unwrap();

        let reps = store.representative_facts_by_slice(m).await.unwrap();
        let mut values: Vec<f64> = reps.iter().map(|f| f.value).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(values, vec![1.0, 3.0], "one rep per distinct dims payload");
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
