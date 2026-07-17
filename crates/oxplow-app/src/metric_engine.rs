//! The metric aggregation engine (epic tsk12, child tsk15) — the heart of
//! "metrics are definitions computed over facts."
//!
//! A metric is NOT a stored row stream; it's a spec (a source measure + an
//! aggregation + an optional filter) evaluated over the durable [`FactRow`]s. The
//! engine turns those facts into a time series (one point per CAPTURE, aggregated
//! ACROSS the subjects measured in that capture) and into a by-dimension rollup.
//!
//! Additivity is respected at the two seams:
//!   * WITHIN a capture, subjects combine per the [`Aggregation`] (sum the
//!     per-file counts, count the matching functions, Σnum/Σden for a ratio).
//!   * ACROSS time, a measure's `temporal_semantics` governs how the series
//!     collapses to one number ([`range_value`]): semi-additive snapshots take the
//!     LAST capture; additive events SUM the captures; ratios re-derive Σnum/Σden.
//!
//! Built additively (no behaviour change): the producers (tsk14) feed facts, the
//! read surface (tsk16) exposes these results. Materialisation into a series cache
//! + derived-metric formulas are follow-ups under tsk15.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_db::{FactRow, MetricCapture, MetricSpec, SqliteFactStore};
use oxplow_domain::{DomainError, Timestamp};

/// How the subjects measured in a single capture combine into the capture's
/// value. (Cross-time collapse is [`range_value`], driven by temporal semantics.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aggregation {
    /// Number of facts (e.g. count of functions over a complexity threshold).
    Count,
    /// Σ of the fact values (e.g. total complexity; a per-file count headline).
    Sum,
    /// Mean of the fact values.
    Avg,
    Min,
    Max,
    /// Value of the last fact in the capture (single-subject/tree metrics).
    Last,
    /// Σnumerator / Σdenominator across the capture's facts (coverage %, pass rate).
    Ratio,
}

impl Aggregation {
    /// Parse the spec string (`sum` | `avg` | `last` | `min` | `max` | `count` |
    /// `ratio`). Unknown ⇒ `None`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "count" => Self::Count,
            "sum" => Self::Sum,
            "avg" => Self::Avg,
            "min" => Self::Min,
            "max" => Self::Max,
            "last" => Self::Last,
            "ratio" => Self::Ratio,
            _ => return None,
        })
    }
}

/// `additive` | `semi-additive` | `non-additive` — a measure's additivity OVER
/// TIME (the BI distinction that governs how a series collapses to one number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Temporal {
    Additive,
    SemiAdditive,
    NonAdditive,
}

impl Temporal {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "additive" => Self::Additive,
            "semi-additive" => Self::SemiAdditive,
            "non-additive" => Self::NonAdditive,
            _ => return None,
        })
    }
}

/// Parse a measure's `temporal_semantics`, naming the measure in the error —
/// malformed additivity is a data-integrity problem, not an empty read.
fn parse_temporal(measure_key: &str, temporal_semantics: &str) -> Result<Temporal, DomainError> {
    Temporal::parse(temporal_semantics).ok_or_else(|| {
        DomainError::Invalid(format!(
            "measure `{measure_key}` has unknown temporal_semantics `{temporal_semantics}`"
        ))
    })
}

/// `complete` | `per-path` — what ONE capture RESTATES (V54, tsk41). A **separate
/// axis** from [`Temporal`]: additivity says how values combine over time,
/// completeness says how much of the population a single capture speaks for.
///
/// The two compose. A tree gauge measure is `semi-additive` (last value per file
/// wins) AND `per-path` (a capture only speaks for the files in its snapshot).
/// Reading it as `semi-additive` alone — "take the last capture" — is what made
/// `oxplow.rust.unsafe_blocks` report 0 while the repo had 15: the last capture
/// was a per-commit delta of 8 files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureScope {
    /// Every capture restates the whole population — a coverage report, a clippy
    /// run, a test run, a whole-tree duplication scan. The temporal fold applies
    /// directly to the per-capture series.
    Complete,
    /// A capture restates only **the paths in its snapshot** (a tree gauge over a
    /// per-commit delta). The value must first be folded to the latest capture per
    /// `(producer, path)` — see `SqliteFactStore::latest_tree_facts` and
    /// [`tree_state_series`].
    PerPath,
    /// A capture restates only **the subjects it emitted facts for** — for
    /// `oxplow.test_case`, the cases the run actually executed (V55, tsk43). The
    /// value is the latest fact per `(producer, subject_ref)`, so a PARTIAL test run
    /// updates just the tests it ran instead of making the metric report a 4-test
    /// repo. Unlike `PerPath` there's no external scanned set (a run has no snapshot
    /// file rows) — the restated set IS the capture's own facts, which is why a
    /// deleted/renamed test lingers. See `SqliteFactStore::latest_subject_facts`.
    PerSubject,
}

impl CaptureScope {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "complete" => Self::Complete,
            "per-path" => Self::PerPath,
            "per-subject" => Self::PerSubject,
            _ => return None,
        })
    }

    /// True when a capture speaks only for part of the population, so the value must
    /// be folded across captures before it's aggregated.
    fn is_partial(self) -> bool {
        matches!(self, Self::PerPath | Self::PerSubject)
    }
}

/// Parse a measure's `capture_scope`, naming the measure in the error.
fn parse_capture_scope(
    measure_key: &str,
    capture_scope: &str,
) -> Result<CaptureScope, DomainError> {
    CaptureScope::parse(capture_scope).ok_or_else(|| {
        DomainError::Invalid(format!(
            "measure `{measure_key}` has unknown capture_scope `{capture_scope}`"
        ))
    })
}

/// A binary op combining two aligned base-metric values into a derived one — the
/// constrained "formula" vocabulary (no general DSL; decision #8). `Div` is the
/// ratio primitive (bugs-per-KLOC, cost-per-token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinaryOp {
    /// Parse the spec string — word or symbol form. `ratio` aliases `div`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "add" | "+" => Self::Add,
            "sub" | "-" => Self::Sub,
            "mul" | "*" => Self::Mul,
            "div" | "/" | "ratio" => Self::Div,
            _ => return None,
        })
    }

    /// Apply to `(a, b)`; `Div` by zero is undefined ⇒ `None` (the derived row is
    /// dropped, never coerced to 0/∞).
    fn apply(self, a: f64, b: f64) -> Option<f64> {
        Some(match self {
            Self::Add => a + b,
            Self::Sub => a - b,
            Self::Mul => a * b,
            Self::Div => {
                if b == 0.0 {
                    return None;
                }
                a / b
            }
        })
    }
}

/// A simple conjunctive predicate over a fact. Covers the common metric filters
/// (count-over-threshold; severity/dimension equality). This is the deserialized
/// shape of a spec's `filter_json`. Richer predicates land with the config layer
/// (tsk17).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactFilter {
    /// Keep facts with `value >= min_value` (e.g. complexity ≥ threshold).
    #[serde(default)]
    pub min_value: Option<f64>,
    /// Keep facts whose reported `severity` equals this (e.g. `error`).
    #[serde(default)]
    pub severity: Option<String>,
    /// Keep facts whose `dims_json[key]` equals this value (a `[key, value]` pair).
    #[serde(default)]
    pub dim_eq: Option<(String, String)>,
}

impl FactFilter {
    /// Parse a spec's `filter_json` into a filter. A malformed predicate is a
    /// config/spec error (surfaced), never silently ignored.
    pub fn from_json(s: &str) -> Result<Self, DomainError> {
        serde_json::from_str(s).map_err(|e| DomainError::Invalid(format!("bad filter_json: {e}")))
    }

    pub fn matches(&self, f: &FactRow) -> bool {
        if let Some(min) = self.min_value {
            if f.value < min {
                return false;
            }
        }
        if let Some(sev) = &self.severity {
            if f.severity.as_deref() != Some(sev.as_str()) {
                return false;
            }
        }
        if let Some((key, val)) = &self.dim_eq {
            if dim_value(f, key).as_deref() != Some(val.as_str()) {
                return false;
            }
        }
        true
    }
}

/// One point in a metric's time series: a single capture's aggregated value.
/// A capture has one branch + one provenance, so a point carries them directly
/// (the read surface renders them without a second lookup, tsk26).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SeriesPoint {
    pub capture_id: i64,
    pub captured_at: Timestamp,
    pub value: f64,
    /// Carried for ratio metrics so downstream roll-ups re-aggregate correctly.
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    /// The group-by dimension value, when the series is sliced by a dimension.
    pub group: Option<String>,
    /// The capture's branch (`None` for operational facts with no worktree).
    pub branch: Option<String>,
    /// The capture's trust label (`observed` | `asserted` | …).
    pub provenance: Option<String>,
    /// The capture's closest git version (short sha), for the recordings table.
    pub git_version: Option<String>,
    /// The capture's collector source (e.g. `nextest`, `agent-reported`).
    pub source: Option<String>,
}

/// One row of a by-dimension rollup (the metric's "breakdown" card).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RollupRow {
    pub key: String,
    pub value: f64,
    pub subject_count: i64,
}

/// A located item behind a metric — the read-time "finding" view over a spec's
/// filtered facts (the offenders drill-in), replacing the baked `metric_finding`
/// (epic tsk12, tsk26). `severity` is the fact's reported severity (lint) or,
/// absent one, DERIVED from the value against the spec's thresholds × direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FactFinding {
    pub subject_kind: Option<String>,
    pub subject_ref: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub value: f64,
    pub severity: Option<String>,
    pub rule: Option<String>,
    pub message: Option<String>,
    pub branch: Option<String>,
    pub captured_at: Timestamp,
}

/// The package (parent directory, repo-relative) of a path; root files ⇒ `"."`.
fn package_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        _ => ".".to_string(),
    }
}

/// Read a conformed-dimension value off a fact: the reported `severity`/`rule`
/// columns for those keys, `package` derived from the path, otherwise the
/// `dims_json` entry under the (namespaced) dimension key.
fn dim_value(f: &FactRow, dimension: &str) -> Option<String> {
    match dimension {
        "oxplow.severity" => f.severity.clone(),
        "oxplow.rule" => f.rule.clone(),
        "oxplow.package" | "package" => {
            let path = f.path.as_deref().or(f.subject_ref.as_deref())?;
            Some(package_of(path))
        }
        // Pseudo-dimensions off the capture/fact spine (not `dims_json`), so
        // `group_by` is uniform server-side (tsk26): branch, the raw subject, and
        // the model (a `model:<id>` subject → the bare id, else the dims_json
        // `oxplow.model`).
        "oxplow.branch" | "branch" => f.branch.clone(),
        // The conformed key is `oxplow.language` (V43), but the Explorer's
        // declared sliceable_dims request the bare form and facts recorded
        // before the gauge scripts namespaced their dims carry bare
        // `language` — both request forms read both fact vintages.
        "oxplow.language" | "language" => {
            // Parse `dims_json` ONCE, then read whichever key vintage is present
            // (conformed `oxplow.language` V43, or the pre-rename bare
            // `language`) — was two parses of the same string (tsk17).
            let dims = parse_dims(f)?;
            dim_from_map(&dims, "oxplow.language").or_else(|| dim_from_map(&dims, "language"))
        }
        "subject" => f.subject_ref.clone(),
        "oxplow.model" | "model" => match &f.subject_ref {
            Some(s) if f.subject_kind.as_deref() == Some("model") => {
                Some(s.strip_prefix("model:").unwrap_or(s).to_string())
            }
            _ => parse_dims(f).and_then(|d| dim_from_map(&d, "oxplow.model")),
        },
        key => parse_dims(f).and_then(|d| dim_from_map(&d, key)),
    }
}

/// Parse a fact's open `dims_json` into its object map — once per lookup.
/// `None` when the fact carries no dims or the JSON isn't an object.
fn parse_dims(f: &FactRow) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::from_str(f.dims_json.as_deref()?).ok()? {
        serde_json::Value::Object(m) => Some(m),
        _ => None,
    }
}

/// A dimension value read from an already-parsed `dims_json` map (the long tail
/// not promoted to a column or a pseudo-dim).
fn dim_from_map(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match map.get(key)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Combine the facts measured in one capture into a `(value, numerator,
/// denominator)` triple per `agg`. `facts` is non-empty.
fn aggregate_facts(facts: &[&FactRow], agg: Aggregation) -> (f64, Option<f64>, Option<f64>) {
    match agg {
        Aggregation::Count => (facts.len() as f64, None, None),
        Aggregation::Sum => (facts.iter().map(|f| f.value).sum(), None, None),
        Aggregation::Avg => {
            let sum: f64 = facts.iter().map(|f| f.value).sum();
            let count = facts.len() as f64;
            // Carry (Σvalues, count) as ratio components so the non-additive
            // cross-time collapse (Σn/Σd in `range_value`) yields the mean
            // across ALL facts — the V47 mean-across-closes measures
            // (cycle_time, task_effort) die to a den=0 → 0.0 otherwise.
            (sum / count, Some(sum), Some(count))
        }
        Aggregation::Min => (
            facts.iter().map(|f| f.value).fold(f64::INFINITY, f64::min),
            None,
            None,
        ),
        Aggregation::Max => (
            facts
                .iter()
                .map(|f| f.value)
                .fold(f64::NEG_INFINITY, f64::max),
            None,
            None,
        ),
        Aggregation::Last => (facts.last().map(|f| f.value).unwrap_or(0.0), None, None),
        Aggregation::Ratio => {
            let mut num = 0.0;
            let mut den = 0.0;
            for f in facts {
                if let (Some(n), Some(d)) = (f.numerator, f.denominator) {
                    num += n;
                    den += d;
                }
            }
            let value = if den != 0.0 { num / den } else { 0.0 };
            (value, Some(num), Some(den))
        }
    }
}

/// The decomposable aggregate of a BUCKET of facts — the cube's stored cell
/// (tsk96). It exists so a read can re-derive an aggregation by **merging cells**
/// instead of re-reading the facts: one `(capture × promoted dims)` cube answers
/// several specs that slice it differently, because every aggregation we serve
/// survives bucketing.
///
/// [`Self::project`] is the counterpart of [`aggregate_facts`] and **must agree
/// with it exactly** — that equivalence is the cube's whole license to exist, and
/// `cube_cells_reaggregate_to_the_same_value_as_the_raw_facts` pins it. Keep the
/// two functions edited together.
///
/// Deliberately NOT stored: anything holding the subject axis. A cell that could
/// answer `min_value >= 11` or `group_by = subject` would have to keep a bucket
/// per distinct value / subject — i.e. be the fact table again. Those reads stay
/// on the facts, by design (see `.context/metrics.md`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cell {
    pub count: i64,
    pub sum: f64,
    /// `None` for an empty cell; the cube never stores one.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Σnumerator / Σdenominator, accumulated ONLY from facts carrying BOTH.
    pub num: f64,
    pub den: f64,
}

impl Cell {
    /// The cell for a bucket of facts.
    pub fn of(facts: &[&FactRow]) -> Self {
        let mut cell = Self::default();
        for f in facts {
            cell.count += 1;
            cell.sum += f.value;
            cell.min = Some(cell.min.map_or(f.value, |m: f64| m.min(f.value)));
            cell.max = Some(cell.max.map_or(f.value, |m: f64| m.max(f.value)));
            // BOTH components or neither — `aggregate_facts` skips a fact with a
            // numerator but no denominator, so a naive Σnumerator would quietly
            // inflate every ratio that has one.
            if let (Some(n), Some(d)) = (f.numerator, f.denominator) {
                cell.num += n;
                cell.den += d;
            }
        }
        cell
    }

    /// Fold another bucket's cell in. Associative and commutative, so the read may
    /// merge the cube's buckets in any order.
    pub fn merge(&mut self, other: &Self) {
        self.count += other.count;
        self.sum += other.sum;
        self.min = match (self.min, other.min) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.max = match (self.max, other.max) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.num += other.num;
        self.den += other.den;
    }

    /// The `(value, numerator, denominator)` triple for `agg` — exactly what
    /// [`aggregate_facts`] returns for the same facts. `None` when `agg` is not
    /// decomposable, which is the read's signal to fall back to the raw facts
    /// rather than guess.
    pub fn project(&self, agg: Aggregation) -> Option<(f64, Option<f64>, Option<f64>)> {
        Some(match agg {
            Aggregation::Count => (self.count as f64, None, None),
            Aggregation::Sum => (self.sum, None, None),
            Aggregation::Avg => {
                let count = self.count as f64;
                // Carry (Σvalues, count) so the cross-time Σn/Σd collapse means
                // the mean across ALL facts — mirrors `aggregate_facts`.
                (self.sum / count, Some(self.sum), Some(count))
            }
            // Empty-cell fallbacks mirror `aggregate_facts`' identity folds.
            Aggregation::Min => (self.min.unwrap_or(f64::INFINITY), None, None),
            Aggregation::Max => (self.max.unwrap_or(f64::NEG_INFINITY), None, None),
            Aggregation::Ratio => {
                let value = if self.den != 0.0 {
                    self.num / self.den
                } else {
                    0.0
                };
                (value, Some(self.num), Some(self.den))
            }
            // "The last fact in the capture" depends on an ordering that merging
            // destroys — there is no bucket-independent answer.
            Aggregation::Last => return None,
        })
    }
}

/// Aggregate facts into a time series: one [`SeriesPoint`] per capture (optionally
/// per capture × dimension value), aggregated across the subjects in that capture.
/// Facts are expected oldest-first (as the store returns them), so the series is
/// time-ascending without a re-sort. Facts failing `filter` are dropped; when
/// `group_by` is set, facts missing that dimension are dropped (they can't be
/// placed on an axis).
pub fn aggregate_series(
    facts: &[FactRow],
    agg: Aggregation,
    filter: &FactFilter,
    group_by: Option<&str>,
) -> Vec<SeriesPoint> {
    // Preserve first-seen (time-ascending) order while grouping.
    let mut order: Vec<(i64, Option<String>)> = Vec::new();
    let mut index: HashMap<(i64, Option<String>), usize> = HashMap::new();
    let mut buckets: Vec<Vec<&FactRow>> = Vec::new();

    for f in facts {
        if !filter.matches(f) {
            continue;
        }
        let group = match group_by {
            Some(dim) => match dim_value(f, dim) {
                Some(g) => Some(g),
                None => continue, // can't place a fact missing the slice dim
            },
            None => None,
        };
        let key = (f.capture_id, group);
        match index.get(&key) {
            Some(&i) => buckets[i].push(f),
            None => {
                index.insert(key.clone(), buckets.len());
                order.push(key);
                buckets.push(vec![f]);
            }
        }
    }

    order
        .into_iter()
        .enumerate()
        .map(|(i, (capture_id, group))| {
            let fs = &buckets[i];
            let captured_at = fs[0].captured_at;
            let (value, numerator, denominator) = aggregate_facts(fs, agg);
            SeriesPoint {
                capture_id,
                captured_at,
                value,
                numerator,
                denominator,
                group,
                // One capture → one branch/provenance/version/source; take the bucket's.
                branch: fs[0].branch.clone(),
                provenance: Some(fs[0].provenance.clone()),
                git_version: fs[0].closest_git_version.clone(),
                source: Some(fs[0].source.clone()),
            }
        })
        .collect()
}

/// The trend for a **`per-path`** measure: the repo-wide value **as of each
/// capture** (V54, tsk41).
///
/// A per-path capture restates only the paths it scanned, so [`aggregate_series`]
/// (one point per capture, over that capture's own facts) would plot *delta-sized*
/// values — "8 files' worth of unsafe blocks" — not the repo. Instead we replay the
/// captures oldest-first, keeping the folded state: at each capture, **evict every
/// path it restated** (scoped to that capture's producer) and **insert whatever
/// facts it emitted** for them, then aggregate the whole live state into the point.
///
/// This is the same fold `latest_tree_facts` does in SQL, run incrementally so each
/// capture yields a point. It falls out of it that:
/// - a file whose count dropped to 0 emits no fact ⇒ evicted, nothing re-inserted;
/// - a **deleted** path is in `scanned` (a deletion IS a scan result) ⇒ evicted;
/// - symbol-grained / many-facts-per-path measures are replaced *wholesale per file*.
///
/// `captures` must be oldest-first and **include empty captures** (a scan that found
/// nothing still restates its paths). `scanned` maps capture_id → the paths it
/// restated.
///
/// Cost is O(captures × live facts) — the aggregate is recomputed per point. That's
/// fine for a chart read; the HEADLINE never comes through here (it's the last point
/// of this series, or the single-shot SQL fold), so the hot path stays cheap.
/// The state key for a fact with no path/subject — an agent-asserted repo scalar
/// (`record_metric` with no subject). Not a real path, so it can never collide with
/// one (paths are never empty and never contain a NUL).
const SCALAR_SUBJECT: &str = "\u{0}repo-scalar";

/// The path a fact is folded under, or `None` when it carries no location at all
/// (a repo-scalar assertion).
fn repo_scalar_key(f: &FactRow) -> Option<&str> {
    f.path.as_deref().or(f.subject_ref.as_deref())
}

pub fn tree_state_series(
    captures: &[MetricCapture],
    facts: &[FactRow],
    scanned: &HashMap<i64, Vec<String>>,
    agg: Aggregation,
    filter: &FactFilter,
    group_by: Option<&str>,
) -> Vec<SeriesPoint> {
    let mut by_capture: HashMap<i64, Vec<&FactRow>> = HashMap::new();
    for f in facts {
        if !filter.matches(f) {
            continue;
        }
        by_capture.entry(f.capture_id).or_default().push(f);
    }

    // The live tree, PER STREAM: stream → (producer, path) → the facts that path
    // currently contributes. Stream keys the state because a stream is a WORKTREE
    // and the fold reconstructs one worktree's tree (tsk98) — two worktrees running
    // the same gauge share `(producer, path)` keys, so a stream-blind state lets one
    // worktree's capture evict the other's paths and yields a point belonging to
    // neither. This mirrors the scoping the fact fetch (tsk75) and the rollup
    // (tsk46) already apply.
    let mut state: HashMap<i64, HashMap<(String, String), Vec<&FactRow>>> = HashMap::new();
    let mut out: Vec<SeriesPoint> = Vec::new();

    for c in captures {
        let tree = state.entry(c.stream_id).or_default();
        for path in scanned.get(&c.id).into_iter().flatten() {
            tree.remove(&(c.producer.clone(), path.clone()));
        }
        let own: Vec<&&FactRow> = by_capture.get(&c.id).into_iter().flatten().collect();
        // A capture may also carry PATH-LESS facts — an agent-asserted repo scalar
        // (`record_metric` with no subject). Those aren't tree facts: no path means the
        // per-path fold can't place or supersede them, so they keep the plain
        // "latest assertion per producer wins" rule. Emitting one restates it.
        if own.iter().any(|f| repo_scalar_key(f).is_none()) {
            tree.remove(&(c.producer.clone(), SCALAR_SUBJECT.to_string()));
        }
        for f in own {
            let key = repo_scalar_key(f).unwrap_or(SCALAR_SUBJECT).to_string();
            tree.entry((c.producer.clone(), key)).or_default().push(f);
        }

        // This capture's OWN stream's tree — an unscoped read replays every
        // stream's captures into one timeline, but each point is still exactly
        // one worktree's state.
        let live: Vec<&FactRow> = tree.values().flatten().copied().collect();
        let point = |value: f64, numerator, denominator, group| SeriesPoint {
            capture_id: c.id,
            captured_at: c.captured_at,
            value,
            numerator,
            denominator,
            group,
            branch: c.branch.clone(),
            provenance: Some(c.provenance.clone()),
            git_version: c.closest_git_version.clone(),
            source: Some(c.source.clone()),
        };

        match group_by {
            None => {
                let (value, numerator, denominator) = if live.is_empty() {
                    (0.0, None, None)
                } else {
                    aggregate_facts(&live, agg)
                };
                out.push(point(value, numerator, denominator, None));
            }
            Some(dim) => {
                // Group the LIVE tree (not the capture's own facts) so each group's
                // point is its repo-wide value at this moment.
                let mut buckets: BTreeMap<String, Vec<&FactRow>> = BTreeMap::new();
                for f in &live {
                    let Some(key) = dim_value(f, dim) else {
                        continue;
                    };
                    buckets.entry(key).or_default().push(f);
                }
                for (key, fs) in buckets {
                    let (value, numerator, denominator) = aggregate_facts(&fs, agg);
                    out.push(point(value, numerator, denominator, Some(key)));
                }
            }
        }
    }
    out
}

/// Collapse a time series to a single in-range number, respecting additivity OVER
/// TIME: a semi-additive snapshot takes the LAST capture (summing snapshots across
/// time double-counts) — re-deriving Σn/Σd from that capture's raw components for a
/// level ratio like coverage (tsk13); an additive event SUMs the captures; a
/// non-additive ratio re-derives Σnumerator / Σdenominator across ALL points (the
/// accumulating mean-across-closes case). `None` for an empty series.
///
/// A `per-path` measure needs no new arm here: [`tree_state_series`] already makes
/// every point the repo-wide value as of that capture, so `SemiAdditive`'s
/// "take the last point" IS the current repo total.
pub fn range_value(series: &[SeriesPoint], temporal: Temporal) -> Option<f64> {
    if series.is_empty() {
        return None;
    }
    Some(match temporal {
        // Level snapshot → the latest capture. For a level RATIO (coverage,
        // tsk13) re-derive from the latest point's RAW Σn/Σd, not its `.value`
        // (which a percent spec has already scaled ×100) — so the spec's
        // presentation scale is applied exactly once downstream, matching the
        // non-additive branch.
        Temporal::SemiAdditive => series
            .last()
            .map(|p| match (p.numerator, p.denominator) {
                (Some(n), Some(d)) if d != 0.0 => n / d,
                _ => p.value,
            })
            .unwrap_or(0.0),
        Temporal::Additive => series.iter().map(|p| p.value).sum(),
        Temporal::NonAdditive => {
            let num: f64 = series.iter().filter_map(|p| p.numerator).sum();
            let den: f64 = series.iter().filter_map(|p| p.denominator).sum();
            if den != 0.0 {
                num / den
            } else {
                0.0
            }
        }
    })
}

/// Roll a measure's facts up by a dimension — the "breakdown" card (which
/// package / language / model holds the most). Additivity-aware per the
/// measure's `temporal_semantics` (the same BI distinction as [`range_value`]):
/// - **semi-additive** (level gauge): only facts from the CURRENT captures
///   (`current_caps` — the latest scan per (stream, producer), see
///   [`current_capture_ids`]), the latest per subject, summed per group.
///   Without the currency scope, a deleted file / renamed symbol keeps its
///   stale last fact contributing to the breakdown forever (tsk44);
/// - **additive** (event): EVERY fact counts — the group value is the running
///   total (tokens by model is a total, not the last turn); `current_caps` is
///   ignored;
/// - **non-additive** (ratio): current captures, latest per subject, then per
///   group Σnumerator/Σdenominator — never a sum (or mean) of percentages.
///   Facts without ratio components fall back to the mean of their values.
///
/// Largest first; ties broken on key for determinism. Facts are expected
/// oldest-first, so the last fact seen per subject is its latest.
pub fn compute_rollup(
    facts: &[FactRow],
    dimension: &str,
    temporal: Temporal,
    current_caps: &std::collections::HashSet<i64>,
) -> Vec<RollupRow> {
    // The facts that contribute: all of them for an additive event measure;
    // the current captures' latest-per-subject otherwise.
    let mut latest: HashMap<String, &FactRow> = HashMap::new();
    let kept: Vec<&FactRow> = match temporal {
        Temporal::Additive => facts
            .iter()
            .filter(|f| f.subject_ref.is_some() || f.path.is_some())
            .collect(),
        Temporal::SemiAdditive | Temporal::NonAdditive => {
            for f in facts {
                if !current_caps.contains(&f.capture_id) {
                    continue;
                }
                let Some(subject) = f.subject_ref.as_deref().or(f.path.as_deref()) else {
                    continue;
                };
                latest.insert(subject.to_string(), f);
            }
            latest.values().copied().collect()
        }
    };

    #[derive(Default)]
    struct Acc {
        value_sum: f64,
        num: f64,
        den: f64,
        fact_count: i64,
        subjects: std::collections::BTreeSet<String>,
    }
    let mut by_key: std::collections::BTreeMap<String, Acc> = std::collections::BTreeMap::new();
    for f in kept {
        let Some(key) = dim_value(f, dimension) else {
            continue;
        };
        let entry = by_key.entry(key).or_default();
        entry.value_sum += f.value;
        entry.num += f.numerator.unwrap_or(0.0);
        entry.den += f.denominator.unwrap_or(0.0);
        entry.fact_count += 1;
        if let Some(subject) = f.subject_ref.as_deref().or(f.path.as_deref()) {
            entry.subjects.insert(subject.to_string());
        }
    }

    let mut out: Vec<RollupRow> = by_key
        .into_iter()
        .map(|(key, acc)| {
            let value = if acc.den != 0.0 {
                // Ratio components present (coverage, pass-rate): re-derive
                // Σnumerator/Σdenominator — never sum or average percentages.
                // Applies to BOTH non-additive accumulating ratios AND
                // semi-additive level ratios like coverage (tsk13).
                acc.num / acc.den
            } else {
                match temporal {
                    // A ratio measure whose facts lack num/den: mean of the
                    // latest per-subject values (defensive — never sum %).
                    Temporal::NonAdditive => acc.value_sum / acc.fact_count as f64,
                    _ => acc.value_sum,
                }
            };
            RollupRow {
                key,
                value,
                subject_count: acc.subjects.len() as i64,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// The CURRENT capture per (stream, producer): the newest capture in the union
/// of the kept facts' captures and the producers' capture rows (which include
/// EMPTY zero-hit captures — so a scan that found nothing empties the rollup
/// rather than leaving the previous scan's values standing, tsk44). A stream is
/// a worktree and a producer is one scan kind, so "the latest scan" is per
/// (stream, producer) — a measure fed by several gauges keeps each gauge's
/// latest capture.
pub fn current_capture_ids(
    kept: &[FactRow],
    captures: &[MetricCapture],
) -> std::collections::HashSet<i64> {
    fn consider(
        best: &mut HashMap<(i64, String), (Timestamp, i64)>,
        stream: i64,
        producer: &str,
        at: Timestamp,
        id: i64,
    ) {
        let candidate = (at, id);
        best.entry((stream, producer.to_string()))
            .and_modify(|cur| {
                if *cur < candidate {
                    *cur = candidate;
                }
            })
            .or_insert(candidate);
    }
    let mut best: HashMap<(i64, String), (Timestamp, i64)> = HashMap::new();
    for f in kept {
        consider(
            &mut best,
            f.stream_id,
            &f.producer,
            f.captured_at,
            f.capture_id,
        );
    }
    for c in captures {
        consider(&mut best, c.stream_id, &c.producer, c.captured_at, c.id);
    }
    best.values().map(|&(_, id)| id).collect()
}

/// Combine two base metrics' by-dimension rollups into a **derived metric**,
/// inner-joining on the shared dimension key. The inner join is what enforces
/// drill-across compatibility (decision #8): a derived value exists only for
/// keys BOTH base metrics carry — a package with LOC but no bugs (or vice-versa)
/// is *dropped*, never silently treated as zero, which would fabricate a ratio.
/// `Div` by zero drops the row (undefined). This is also the server-side home of
/// the Explorer's `buildScatterPoints` pairing: roll each metric up by the same
/// grain (subject or dimension), then pair here. The result keeps `left`'s
/// `subject_count` (the derived metric reads "left per right") and sorts
/// largest-value first, ties on key — matching [`compute_rollup`].
pub fn evaluate_formula(left: &[RollupRow], right: &[RollupRow], op: BinaryOp) -> Vec<RollupRow> {
    let right_by: HashMap<&str, f64> = right.iter().map(|r| (r.key.as_str(), r.value)).collect();
    let mut out: Vec<RollupRow> = left
        .iter()
        .filter_map(|l| {
            let rv = right_by.get(l.key.as_str())?;
            let value = op.apply(l.value, *rv)?;
            Some(RollupRow {
                key: l.key.clone(),
                value,
                subject_count: l.subject_count,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// The engine over a fact store: fetches the right facts and applies the pure
/// aggregation. A metric's facts are fetched by its `source_measure`.
#[derive(Clone)]
pub struct MetricEngine {
    facts: SqliteFactStore,
}

impl MetricEngine {
    pub fn new(facts: SqliteFactStore) -> Self {
        Self { facts }
    }

    /// The time series for a measure under an aggregation + filter, optionally
    /// sliced by a conformed dimension. Empty when the measure is unknown.
    /// Count/sum series are ZERO-FILLED from the producers' captures (tsk44):
    /// a scan that found nothing writes an EMPTY capture, and the fill turns it
    /// into an explicit value-0 point so the metric drops back to zero.
    pub async fn series(
        &self,
        measure_key: &str,
        agg: Aggregation,
        filter: &FactFilter,
        group_by: Option<&str>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        self.series_in_stream(measure_key, agg, filter, group_by, None)
            .await
    }

    /// [`Self::series`] scoped to one stream (worktree) — per-worktree scans
    /// don't interleave into one timeline (the series sibling of the tsk46
    /// rollup scoping). `None` reads across all streams.
    pub async fn series_in_stream(
        &self,
        measure_key: &str,
        agg: Aggregation,
        filter: &FactFilter,
        group_by: Option<&str>,
        stream: Option<i64>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        // A trend legitimately reads the measure's history (one point per
        // capture) — but stream-bounded SQL-side (tsk75), never loaded whole
        // then filtered in Rust.
        let facts: Vec<FactRow> = match stream {
            Some(s) => {
                self.facts
                    .facts_for_measure_in_stream(measure.id, s)
                    .await?
            }
            None => self.facts.facts_for_measure(measure.id).await?,
        };

        // The producers whose scans emit this metric's slice — derived from the
        // facts that (ever) matched the filter, so an unrelated producer's
        // captures never inject zero points.
        let mut producers: std::collections::BTreeSet<String> = facts
            .iter()
            .filter(|f| filter.matches(f))
            .map(|f| f.producer.clone())
            .collect();
        // Chicken-and-egg for the analysis measure (tsk62): an analyzer that
        // has been CLEAN since day one has zero lint_hit facts, so the
        // fact-derived set is empty and its "ran, found nothing" captures
        // could never zero-fill — `oxplow.analysis.*` read blank forever
        // instead of 0. Analyzer names are dynamic (clippy/eslint/…), but
        // their report-tier captures all carry `source = 'analysis-report'`,
        // so seed from those. Measure-keyed on purpose — no other built-in
        // measure has a "clean forever" empty-capture semantics to surface.
        if producers.is_empty() && measure_key == "oxplow.lint_hit" {
            producers.extend(
                self.facts
                    .producers_for_capture_source("analysis-report")
                    .await?,
            );
        }

        // A PARTIAL-capture measure's captures each speak for only part of the
        // population — a `per-path` tree gauge's snapshot delta, or a `per-subject`
        // test run's executed cases. One point per capture would plot "the files in
        // that commit" / "the tests in that run", not the repo. Replay them into a
        // running state instead (tsk41/tsk43). The two differ only in WHAT a capture
        // restates: the snapshot's file rows, or the capture's own subjects.
        //
        // Zero-fill is deliberately NOT applied: an empty capture restated nothing, so
        // it means "nothing changed", not "the repo is zero" — zero-filling would yank
        // the headline to 0.
        let scope = parse_capture_scope(measure_key, &measure.capture_scope)?;
        if scope.is_partial() {
            let captures = self
                .facts
                .captures_for_producers(producers.into_iter().collect())
                .await?
                .into_iter()
                .filter(|c| stream.map_or(true, |s| c.stream_id == s))
                .collect::<Vec<_>>();
            let restated = match scope {
                CaptureScope::PerPath => self.scanned_paths(&captures).await?,
                // A test run restates exactly the cases it executed.
                _ => {
                    let mut m: HashMap<i64, Vec<String>> = HashMap::new();
                    for f in &facts {
                        if let Some(s) = f.subject_ref.as_deref() {
                            m.entry(f.capture_id).or_default().push(s.to_string());
                        }
                    }
                    m
                }
            };
            return Ok(tree_state_series(
                &captures, &facts, &restated, agg, filter, group_by,
            ));
        }

        let points = aggregate_series(&facts, agg, filter, group_by);
        self.zero_fill(points, producers, agg, group_by, stream)
            .await
    }

    /// `capture_id → the paths that capture restated` (its snapshot's file rows,
    /// deletion tombstones included). The scanned set for the per-path fold.
    async fn scanned_paths(
        &self,
        captures: &[MetricCapture],
    ) -> Result<HashMap<i64, Vec<String>>, DomainError> {
        let ids: Vec<i64> = captures.iter().map(|c| c.id).collect();
        let mut out: HashMap<i64, Vec<String>> = HashMap::new();
        for (capture_id, path) in self.facts.scanned_paths_for_captures(ids).await? {
            out.entry(capture_id).or_default().push(path);
        }
        Ok(out)
    }

    /// Splice value-0 points into `points` for every capture of `producers`
    /// that produced none (the empty "scanned, found nothing" record — tsk44).
    /// Only count/sum aggregations have a meaningful zero (avg/min/max/last/
    /// ratio of nothing is undefined → left sparse), and only ungrouped series
    /// are filled (an empty capture carries no group values). A stream-scoped
    /// series only fills from THAT stream's captures — another worktree's
    /// zero-hit scan is not this timeline's zero.
    async fn zero_fill(
        &self,
        mut points: Vec<SeriesPoint>,
        producers: std::collections::BTreeSet<String>,
        agg: Aggregation,
        group_by: Option<&str>,
        stream: Option<i64>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        if group_by.is_some()
            || !matches!(agg, Aggregation::Count | Aggregation::Sum)
            || producers.is_empty()
        {
            return Ok(points);
        }
        let caps = self
            .facts
            .captures_for_producers(producers.into_iter().collect())
            .await?;
        let have: std::collections::HashSet<i64> = points.iter().map(|p| p.capture_id).collect();
        for c in caps {
            if stream.is_some_and(|s| c.stream_id != s) {
                continue;
            }
            if !have.contains(&c.id) {
                points.push(SeriesPoint {
                    capture_id: c.id,
                    captured_at: c.captured_at,
                    value: 0.0,
                    numerator: None,
                    denominator: None,
                    group: None,
                    branch: c.branch.clone(),
                    provenance: Some(c.provenance.clone()),
                    git_version: c.closest_git_version.clone(),
                    source: Some(c.source.clone()),
                });
            }
        }
        points.sort_by(|a, b| {
            a.captured_at
                .cmp(&b.captured_at)
                .then(a.capture_id.cmp(&b.capture_id))
        });
        Ok(points)
    }

    /// The by-dimension rollup (breakdown) for a measure, additivity-aware per
    /// its `temporal_semantics` and scoped to the CURRENT captures (see
    /// [`compute_rollup`] / [`current_capture_ids`]). Empty when unknown.
    pub async fn rollup(
        &self,
        measure_key: &str,
        dimension: &str,
    ) -> Result<Vec<RollupRow>, DomainError> {
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        let temporal = parse_temporal(measure_key, &measure.temporal_semantics)?;
        let facts = self.scoped_facts(&measure, None).await?;
        let current = self.currency(&measure, &facts).await?;
        Ok(compute_rollup(&facts, dimension, temporal, &current))
    }

    /// The facts a point-in-time read (rollup / findings) should consider, per the
    /// measure's `capture_scope`. For `per-path` this is the SQL tree fold — the
    /// latest capture per (producer, path) — so the read sees the whole repo, not
    /// the last commit's files. For `complete` it's the measure's facts as before.
    async fn scoped_facts(
        &self,
        measure: &oxplow_db::Measure,
        stream: Option<i64>,
    ) -> Result<Vec<FactRow>, DomainError> {
        let scope = parse_capture_scope(&measure.key, &measure.capture_scope)?;
        if scope == CaptureScope::PerSubject {
            // The restated set is the capture's own facts, so the fold is simply the
            // latest fact per subject — a partial test run updates only the tests it ran.
            return self.facts.latest_subject_facts(measure.id, stream).await;
        }
        if scope == CaptureScope::PerPath {
            let mut folded = self.facts.latest_tree_facts(measure.id, stream).await?;
            // The SQL fold joins on `path`, so PATH-LESS facts — an agent-asserted repo
            // scalar (`record_metric` with no subject) — are not in it. They aren't tree
            // facts: with no path there is nothing to supersede them per-path, so they
            // keep the plain per-producer currency rule (latest assertion wins). Without
            // this an asserted number would silently vanish from every read. SQL-side
            // (tsk75): this used to load the measure's ENTIRE history to find the
            // usually-zero scalars.
            let scalars: Vec<FactRow> =
                self.facts.pathless_scalar_facts(measure.id, stream).await?;
            if !scalars.is_empty() {
                let current = self.current_captures(&scalars).await?;
                folded.extend(
                    scalars
                        .into_iter()
                        .filter(|f| current.contains(&f.capture_id)),
                );
            }
            return Ok(folded);
        }
        Ok(self
            .facts
            .facts_for_measure(measure.id)
            .await?
            .into_iter()
            .filter(|f| stream.map_or(true, |s| f.stream_id == s))
            .collect())
    }

    /// The currency gate [`compute_rollup`] should apply, per `capture_scope`.
    ///
    /// - `complete`: every capture restates the whole population, so "current" is
    ///   the latest capture per (stream, producer) — the existing gate (tsk44).
    /// - `per-path`: currency is *per path*, not per capture, and the fold has
    ///   already applied it — every fact it returned IS current. So the gate must
    ///   be a no-op; applying the capture-level gate here would (wrongly) keep only
    ///   the newest delta's facts and re-introduce the bug.
    async fn currency(
        &self,
        measure: &oxplow_db::Measure,
        kept: &[FactRow],
    ) -> Result<std::collections::HashSet<i64>, DomainError> {
        if parse_capture_scope(&measure.key, &measure.capture_scope)?.is_partial() {
            return Ok(kept.iter().map(|f| f.capture_id).collect());
        }
        self.current_captures(kept).await
    }

    /// The current-capture set for a fact slice: the latest capture per
    /// (stream, producer), including the producers' EMPTY zero-hit captures
    /// (see [`current_capture_ids`], tsk44).
    async fn current_captures(
        &self,
        kept: &[FactRow],
    ) -> Result<std::collections::HashSet<i64>, DomainError> {
        let producers: std::collections::BTreeSet<String> =
            kept.iter().map(|f| f.producer.clone()).collect();
        let caps = self
            .facts
            .captures_for_producers(producers.into_iter().collect())
            .await?;
        Ok(current_capture_ids(kept, &caps))
    }

    // --- spec-driven reads (a metric key → its computed result) -----------

    /// The time series for a metric SPEC: resolve its `source_measure`,
    /// `aggregation`, and `filter`, then aggregate the source measure's facts.
    /// Empty for a formula metric (no source measure) or an unknown measure.
    /// Errors on an aggregation the engine can't yet compute or a malformed
    /// `filter_json`.
    pub async fn series_for_spec(
        &self,
        spec: &MetricSpec,
        group_by: Option<&str>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        self.series_for_spec_in_stream(spec, group_by, None).await
    }

    /// [`Self::series_for_spec`] scoped to one stream (worktree) — see
    /// [`Self::series_in_stream`]. `None` reads across all streams.
    pub async fn series_for_spec_in_stream(
        &self,
        spec: &MetricSpec,
        group_by: Option<&str>,
        stream: Option<i64>,
    ) -> Result<Vec<SeriesPoint>, DomainError> {
        let Some(measure_key) = spec.source_measure.as_deref() else {
            return Ok(Vec::new());
        };
        let agg = spec_aggregation(spec)?;
        let filter = spec_filter(spec)?;
        let mut series = self
            .series_in_stream(measure_key, agg, &filter, group_by, stream)
            .await?;
        let scale = spec_value_scale(spec);
        if scale != 1.0 {
            for p in &mut series {
                // Present on the spec's scale; the raw components stay for
                // downstream re-aggregation.
                p.value *= scale;
            }
        }
        Ok(series)
    }

    /// The by-dimension rollup for a spec — the source measure's facts filtered by
    /// the spec's predicate, then rolled up by `dimension` additivity-aware per
    /// the measure's `temporal_semantics` (see [`compute_rollup`]). Empty for a
    /// formula / unknown-measure spec.
    pub async fn rollup_for_spec(
        &self,
        spec: &MetricSpec,
        dimension: &str,
    ) -> Result<Vec<RollupRow>, DomainError> {
        self.rollup_for_spec_in_stream(spec, dimension, None).await
    }

    /// [`rollup_for_spec`] scoped to one stream (worktree) — a per-worktree
    /// breakdown that doesn't mix another stream's scans (tsk46). `None` rolls
    /// up across all streams.
    pub async fn rollup_for_spec_in_stream(
        &self,
        spec: &MetricSpec,
        dimension: &str,
        stream: Option<i64>,
    ) -> Result<Vec<RollupRow>, DomainError> {
        let Some(measure_key) = spec.source_measure.as_deref() else {
            return Ok(Vec::new());
        };
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        let temporal = parse_temporal(measure_key, &measure.temporal_semantics)?;
        let filter = spec_filter(spec)?;
        // `scoped_facts` resolves the capture_scope axis: for a per-path measure it
        // returns the tree-folded facts (latest capture per (producer, path)), so a
        // breakdown reads the whole repo rather than the last commit's files.
        let facts = self.scoped_facts(&measure, stream).await?;
        let kept: Vec<FactRow> = facts.into_iter().filter(|f| filter.matches(f)).collect();
        let current = self.currency(&measure, &kept).await?;
        let mut rollup = compute_rollup(&kept, dimension, temporal, &current);
        let scale = spec_value_scale(spec);
        if scale != 1.0 {
            for r in &mut rollup {
                r.value *= scale;
            }
        }
        Ok(rollup)
    }

    /// The single headline number for a spec: its series collapsed across TIME per
    /// the source measure's `temporal_semantics` (semi-additive → last capture;
    /// additive → sum; ratio → Σn/Σd). `None` for a formula / unknown / empty spec.
    pub async fn headline_for_spec(&self, spec: &MetricSpec) -> Result<Option<f64>, DomainError> {
        self.headline_for_spec_in_stream(spec, None).await
    }

    /// [`Self::headline_for_spec`] scoped to one stream (worktree): the collapse
    /// runs over that stream's series, so a semi-additive headline is ITS last
    /// scan — not whichever worktree scanned most recently. `None` = all streams.
    pub async fn headline_for_spec_in_stream(
        &self,
        spec: &MetricSpec,
        stream: Option<i64>,
    ) -> Result<Option<f64>, DomainError> {
        let series = self.series_for_spec_in_stream(spec, None, stream).await?;
        self.headline_from_series(spec, &series).await
    }

    /// Collapse an ALREADY-COMPUTED (ungrouped) series to the spec's headline per
    /// its source measure's `temporal_semantics` — [`Self::headline_for_spec`]
    /// without paying the fact load + aggregation a second time (the summary
    /// reads compute the series anyway).
    pub async fn headline_from_series(
        &self,
        spec: &MetricSpec,
        series: &[SeriesPoint],
    ) -> Result<Option<f64>, DomainError> {
        let Some(measure_key) = spec.source_measure.as_deref() else {
            return Ok(None);
        };
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(None);
        };
        let temporal = parse_temporal(measure_key, &measure.temporal_semantics)?;
        // A ratio collapse (semi- or non-additive) re-derives from the RAW
        // components, so the spec's presentation scale applies exactly once
        // here — `range_value` never reads an already-scaled `.value` for a
        // measure carrying num/den.
        Ok(range_value(series, temporal).map(|v| v * spec_value_scale(spec)))
    }

    /// The located items behind a spec — its filtered facts projected as
    /// [`FactFinding`]s (the offenders drill-in that replaces the baked
    /// `metric_finding`). `capture_id` scopes to one capture (a recording's
    /// drill-in); `None` returns every matching fact. Empty for a formula /
    /// unknown-measure spec. Severity is the fact's reported severity or, absent
    /// one, derived from the value × the spec's thresholds × direction.
    pub async fn findings_for_spec(
        &self,
        spec: &MetricSpec,
        capture_id: Option<i64>,
    ) -> Result<Vec<FactFinding>, DomainError> {
        let Some(measure_key) = spec.source_measure.as_deref() else {
            return Ok(Vec::new());
        };
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(Vec::new());
        };
        let filter = spec_filter(spec)?;
        // Scope to the CURRENT tree for a per-path measure. Without this the
        // drill-in lists every historical fact for a file — including the ones a
        // later rescan superseded — so a fixed `unsafe` block would keep showing up
        // forever. (Pinning an explicit `capture_id` still reads that one capture.)
        let facts = match capture_id {
            // Pinned drill-in: exactly that capture's facts — never the whole
            // measure history filtered in Rust (tsk75).
            Some(c) => self.facts.facts_for_captures(measure.id, vec![c]).await?,
            None => self.scoped_facts(&measure, None).await?,
        };
        Ok(facts
            .into_iter()
            .filter(|f| filter.matches(f))
            .filter(|f| match capture_id {
                Some(c) => f.capture_id == c,
                None => true,
            })
            .map(|f| {
                let severity = f.severity.clone().or_else(|| {
                    threshold_state(&spec.direction, f.value, spec.warn_at, spec.fail_at)
                        .map(str::to_string)
                });
                FactFinding {
                    subject_kind: f.subject_kind,
                    subject_ref: f.subject_ref,
                    path: f.path,
                    line: f.line,
                    value: f.value,
                    severity,
                    rule: f.rule,
                    message: f.detail,
                    branch: f.branch,
                    captured_at: f.captured_at,
                }
            })
            .collect())
    }
}

/// Classify a value against a metric's thresholds, interpreted via `direction`.
/// Returns `Some("fail")` / `Some("warn")` when the value is in that zone, else
/// `None`. `neutral` metrics (no better/worse) never cross. The worse side is
/// "higher" for `lower-better` and "lower" for `higher-better`. Shared by the
/// legacy effort-panel read (`collection.rs`) and the fact finding view.
pub fn threshold_state(
    direction: &str,
    value: f64,
    warn_at: Option<f64>,
    fail_at: Option<f64>,
) -> Option<&'static str> {
    let worse_when_above = match direction {
        "lower-better" => true,
        "higher-better" => false,
        // neutral / unknown: no threshold semantics.
        _ => return None,
    };
    let crosses = |t: f64| {
        if worse_when_above {
            value >= t
        } else {
            value <= t
        }
    };
    if let Some(f) = fail_at {
        if crosses(f) {
            return Some("fail");
        }
    }
    if let Some(w) = warn_at {
        if crosses(w) {
            return Some("warn");
        }
    }
    None
}

/// Map a spec's `aggregation` string onto the engine's [`Aggregation`]. The spec
/// vocabulary is a superset (`count_distinct`/`p95` are reserved in the schema);
/// an aggregation the engine can't yet compute is an honest error, not a silent
/// wrong number.
fn spec_aggregation(spec: &MetricSpec) -> Result<Aggregation, DomainError> {
    Aggregation::parse(&spec.aggregation).ok_or_else(|| {
        DomainError::Invalid(format!(
            "aggregation `{}` is not yet supported by the engine",
            spec.aggregation
        ))
    })
}

/// The presentation scale for a spec's aggregated values: a `ratio` spec with
/// unit `%` reads ×100. The facts carry raw components (covered/instrumented
/// lines) and the engine derives a 0..1 fraction, but the spec's unit,
/// target/warn/fail thresholds, and the per-fact `value` column are all on the
/// 0..100 scale — the spec-driven reads must agree with them, or 85% coverage
/// renders as an always-failing "0.85%" (tsk3). Applies only where the spec is
/// in scope; measure-level reads return the raw fraction.
fn spec_value_scale(spec: &MetricSpec) -> f64 {
    if spec.aggregation == "ratio" && spec.unit.as_deref() == Some("%") {
        100.0
    } else {
        1.0
    }
}

/// A spec's `filter_json` as a [`FactFilter`] (the empty filter when absent).
fn spec_filter(spec: &MetricSpec) -> Result<FactFilter, DomainError> {
    match spec.filter_json.as_deref() {
        Some(j) => FactFilter::from_json(j),
        None => Ok(FactFilter::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    /// A fact in `capture_id` captured at `captured_at` with `value`; override the
    /// remaining fields with struct-update syntax in each test as needed.
    fn fact(capture_id: i64, captured_at: &str, value: f64) -> FactRow {
        FactRow {
            id: 0,
            capture_id,
            measure_id: 1,
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
            captured_at: ts(captured_at),
            branch: None,
            closest_git_version: None,
            git_version_exact: false,
            basis_ref: None,
            snapshot_id: None,
            stream_id: 1,
            thread_id: None,
            effort_id: None,
            provenance: "observed".into(),
            source: "test".into(),
            producer: "test.gauge".into(),
        }
    }

    #[test]
    fn cube_cells_reaggregate_to_the_same_value_as_the_raw_facts() {
        // tsk96: the cube can only exist if every aggregation it serves is
        // DECOMPOSABLE — bucket the facts, aggregate each bucket, merge the
        // buckets, and you must land on exactly what aggregating all the facts
        // at once gives. This is the proof, and it is what lets one
        // (capture × dims) cube answer several specs that slice it differently.
        //
        // Values are chosen to be exactly representable in binary so `==` on the
        // f64s is a real assertion about the arithmetic and not a rounding race.
        let ratio_fact = |value: f64, num: Option<f64>, den: Option<f64>| FactRow {
            numerator: num,
            denominator: den,
            ..fact(1, "2026-06-30T10:00:00Z", value)
        };
        let facts = [
            ratio_fact(2.0, Some(3.0), Some(4.0)),
            ratio_fact(7.0, Some(1.0), Some(5.0)),
            // A numerator with NO denominator: `aggregate_facts` skips it
            // entirely, so a cell that naively accumulated Σnumerator would
            // report 13/9 here instead of 4/9 — a wrong coverage %.
            ratio_fact(5.0, Some(9.0), None),
            ratio_fact(-1.0, None, None),
        ];
        let all: Vec<&FactRow> = facts.iter().collect();
        // An arbitrary, uneven partition — the cube's buckets are whatever the
        // promoted dims happen to carve out, so nothing may depend on the split.
        let buckets: [Vec<&FactRow>; 3] =
            [vec![&facts[0], &facts[2]], vec![&facts[1]], vec![&facts[3]]];

        for agg in [
            Aggregation::Count,
            Aggregation::Sum,
            Aggregation::Avg,
            Aggregation::Min,
            Aggregation::Max,
            Aggregation::Ratio,
        ] {
            let mut merged = Cell::default();
            for bucket in &buckets {
                merged.merge(&Cell::of(bucket));
            }
            assert_eq!(
                merged.project(agg),
                Some(aggregate_facts(&all, agg)),
                "{agg:?} must survive bucketing + merging"
            );
        }
        // `Last` ("the last fact in the capture") has no bucket-independent
        // answer — merging loses the ordering it depends on. It must refuse,
        // not guess, so the read falls back to the raw facts.
        assert_eq!(Cell::of(&all).project(Aggregation::Last), None);
    }

    #[test]
    fn count_series_counts_facts_over_threshold_per_capture() {
        // Capture 1 (10:00): three functions, two over threshold 10.
        // Capture 2 (11:00): one over threshold.
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 14.0),
            fact(1, "2026-06-30T10:00:00Z", 11.0),
            fact(1, "2026-06-30T10:00:00Z", 3.0),
            fact(2, "2026-06-30T11:00:00Z", 20.0),
            fact(2, "2026-06-30T11:00:00Z", 2.0),
        ];
        let series = aggregate_series(
            &facts,
            Aggregation::Count,
            &FactFilter {
                min_value: Some(10.0),
                ..Default::default()
            },
            None,
        );
        // One point per capture, time-ascending: 2 over-threshold, then 1.
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![2.0, 1.0]
        );
        // A count metric is semi-additive: the in-range value is the LAST capture.
        assert_eq!(range_value(&series, Temporal::SemiAdditive), Some(1.0));
    }

    #[test]
    fn sum_series_totals_per_capture_subjects() {
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 14.0),
            fact(1, "2026-06-30T10:00:00Z", 6.0),
        ];
        let series = aggregate_series(&facts, Aggregation::Sum, &FactFilter::default(), None);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].value, 20.0);
    }

    #[test]
    fn ratio_series_reaggregates_components_not_naive_average() {
        // File A 1/1 (100%), File B 0/3 (0%). True combined = 1/4 = 25%, not 50%.
        let facts = vec![
            FactRow {
                numerator: Some(1.0),
                denominator: Some(1.0),
                subject_ref: Some("a.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 1.0)
            },
            FactRow {
                numerator: Some(0.0),
                denominator: Some(3.0),
                subject_ref: Some("b.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 0.0)
            },
        ];
        let series = aggregate_series(&facts, Aggregation::Ratio, &FactFilter::default(), None);
        assert_eq!(series.len(), 1);
        assert!(
            (series[0].value - 0.25).abs() < 1e-9,
            "got {}",
            series[0].value
        );
        // Non-additive across time too: Σnum/Σden, never an average of ratios.
        assert_eq!(range_value(&series, Temporal::NonAdditive), Some(0.25));
    }

    #[test]
    fn range_value_semi_additive_ratio_reads_the_latest_capture_not_a_blend() {
        // tsk13: coverage is a semi-additive LEVEL ratio. Capture 1 = 50%
        // (1/2), capture 2 = 85% (17/20). The in-range headline is the
        // latest capture (0.85), NOT the Σn/Σd blend across both (18/22 ≈
        // 0.818) that non-additive accumulation gives.
        let facts = vec![
            FactRow {
                numerator: Some(1.0),
                denominator: Some(2.0),
                subject_ref: Some("a.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 0.5)
            },
            FactRow {
                numerator: Some(17.0),
                denominator: Some(20.0),
                subject_ref: Some("a.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 0.85)
            },
        ];
        let series = aggregate_series(&facts, Aggregation::Ratio, &FactFilter::default(), None);
        assert_eq!(series.len(), 2);
        assert_eq!(range_value(&series, Temporal::SemiAdditive), Some(0.85));
        // Contrast: non-additive accumulation blends across all captures.
        let blend = range_value(&series, Temporal::NonAdditive).unwrap();
        assert!((blend - 18.0 / 22.0).abs() < 1e-9, "got {blend}");
    }

    #[test]
    fn additive_series_sums_across_time_in_range() {
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 100.0),
            fact(2, "2026-06-30T11:00:00Z", 250.0),
        ];
        let series = aggregate_series(&facts, Aggregation::Sum, &FactFilter::default(), None);
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![100.0, 250.0]
        );
        // Additive over time: the in-range value SUMS the captures (350), unlike a
        // semi-additive snapshot which would take just the last (250).
        assert_eq!(range_value(&series, Temporal::Additive), Some(350.0));
        assert_eq!(range_value(&series, Temporal::SemiAdditive), Some(250.0));
    }

    #[test]
    fn series_grouped_by_dimension_splits_per_value() {
        let facts = vec![
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"rust\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 10.0)
            },
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"rust\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 4.0)
            },
            FactRow {
                dims_json: Some("{\"oxplow.language\":\"typescript\"}".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 7.0)
            },
        ];
        let mut series = aggregate_series(
            &facts,
            Aggregation::Sum,
            &FactFilter::default(),
            Some("oxplow.language"),
        );
        series.sort_by(|a, b| a.group.cmp(&b.group));
        let by_group: Vec<(Option<String>, f64)> =
            series.iter().map(|p| (p.group.clone(), p.value)).collect();
        assert_eq!(
            by_group,
            vec![
                (Some("rust".to_string()), 14.0),
                (Some("typescript".to_string()), 7.0),
            ]
        );
    }

    #[test]
    fn rollup_sums_latest_per_subject_by_package() {
        // Semi-additive (level gauge): a.rs measured twice — only the latest
        // (3.0) counts; summing snapshots across time would double-count.
        let facts = vec![
            FactRow {
                subject_ref: Some("src/app/a.rs".into()),
                path: Some("src/app/a.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 9.0)
            },
            FactRow {
                subject_ref: Some("src/app/a.rs".into()),
                path: Some("src/app/a.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 3.0)
            },
            FactRow {
                subject_ref: Some("src/app/b.rs".into()),
                path: Some("src/app/b.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 4.0)
            },
            FactRow {
                subject_ref: Some("src/util/c.rs".into()),
                path: Some("src/util/c.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 20.0)
            },
        ];
        let rollup = compute_rollup(
            &facts,
            "package",
            Temporal::SemiAdditive,
            &current_capture_ids(&facts, &[]),
        );
        let rows: Vec<(String, f64, i64)> = rollup
            .iter()
            .map(|r| (r.key.clone(), r.value, r.subject_count))
            .collect();
        // src/util (20) > src/app (3 latest a.rs + 4 b.rs = 7).
        assert_eq!(
            rows,
            vec![
                ("src/util".to_string(), 20.0, 1),
                ("src/app".to_string(), 7.0, 2),
            ]
        );
    }

    #[test]
    fn rollup_drops_subjects_missing_from_the_current_capture() {
        // tsk44: a deleted file's facts stop at an older capture; scoping the
        // semi-additive collapse to the CURRENT capture per (stream, producer)
        // keeps its stale value out of the breakdown forever-after.
        let facts = vec![
            FactRow {
                subject_ref: Some("src/app/old.rs".into()),
                path: Some("src/app/old.rs".into()),
                ..fact(1, "2026-06-30T10:00:00Z", 4.0)
            },
            FactRow {
                subject_ref: Some("src/app/kept.rs".into()),
                path: Some("src/app/kept.rs".into()),
                ..fact(2, "2026-06-30T11:00:00Z", 1.0)
            },
        ];
        let rollup = compute_rollup(
            &facts,
            "package",
            Temporal::SemiAdditive,
            &current_capture_ids(&facts, &[]),
        );
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].key, "src/app");
        assert_eq!(rollup[0].value, 1.0, "old.rs (deleted) contributes nothing");
        assert_eq!(rollup[0].subject_count, 1);
    }

    #[test]
    fn rollup_sums_every_fact_for_an_additive_measure() {
        // Additive (event measure, e.g. tokens): every capture's facts count —
        // latest-per-subject would report only the most recent event, not the
        // total ("tokens by model" must be the running total, not the last turn).
        let by_model = |cap: i64, at: &str, model: &str, v: f64| FactRow {
            subject_kind: Some("model".into()),
            subject_ref: Some(format!("model:{model}")),
            ..fact(cap, at, v)
        };
        let facts = vec![
            by_model(1, "2026-06-30T10:00:00Z", "opus", 100.0),
            by_model(2, "2026-06-30T11:00:00Z", "opus", 50.0),
            by_model(2, "2026-06-30T11:00:00Z", "haiku", 30.0),
        ];
        let rollup = compute_rollup(&facts, "subject", Temporal::Additive, &Default::default());
        let rows: Vec<(String, f64, i64)> = rollup
            .iter()
            .map(|r| (r.key.clone(), r.value, r.subject_count))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("model:opus".to_string(), 150.0, 1),
                ("model:haiku".to_string(), 30.0, 1),
            ]
        );
    }

    #[test]
    fn rollup_rederives_ratio_for_a_non_additive_measure() {
        // Non-additive (ratio, e.g. coverage): per group the value is
        // Σnumerator/Σdenominator over the latest fact per subject — never a sum
        // (or mean) of percentages, which weights a 10-line file like a
        // 1000-line one.
        let cov = |cap: i64, at: &str, p: &str, num: f64, den: f64| FactRow {
            subject_ref: Some(p.to_string()),
            path: Some(p.to_string()),
            numerator: Some(num),
            denominator: Some(den),
            ..fact(cap, at, if den != 0.0 { num / den } else { 0.0 })
        };
        let facts = vec![
            // a.rs re-measured — only the latest (80/100) counts.
            cov(1, "2026-06-30T10:00:00Z", "src/app/a.rs", 10.0, 100.0),
            cov(2, "2026-06-30T11:00:00Z", "src/app/a.rs", 80.0, 100.0),
            cov(2, "2026-06-30T11:00:00Z", "src/app/b.rs", 5.0, 10.0),
            cov(2, "2026-06-30T11:00:00Z", "src/util/c.rs", 9.0, 10.0),
        ];
        let rollup = compute_rollup(
            &facts,
            "package",
            Temporal::NonAdditive,
            &current_capture_ids(&facts, &[]),
        );
        let rows: Vec<(String, f64, i64)> = rollup
            .iter()
            .map(|r| (r.key.clone(), r.value, r.subject_count))
            .collect();
        // src/util: 9/10 = 0.9 > src/app: (80+5)/(100+10) ≈ 0.7727.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "src/util");
        assert_eq!(rows[0].1, 0.9);
        assert_eq!(rows[0].2, 1);
        assert_eq!(rows[1].0, "src/app");
        assert!((rows[1].1 - 85.0 / 110.0).abs() < 1e-9);
        assert_eq!(rows[1].2, 2);
    }

    #[test]
    fn rollup_rederives_ratio_for_a_semi_additive_ratio_measure() {
        // Coverage is a semi-additive LEVEL ratio (tsk13): the per-package
        // breakdown must re-derive Σn/Σd over the current capture, never sum
        // the per-file percentages (which would exceed 1.0 for two files).
        let cov = |cap: i64, at: &str, p: &str, num: f64, den: f64| FactRow {
            subject_ref: Some(p.to_string()),
            path: Some(p.to_string()),
            numerator: Some(num),
            denominator: Some(den),
            ..fact(cap, at, if den != 0.0 { num / den } else { 0.0 })
        };
        let facts = vec![
            cov(1, "2026-06-30T11:00:00Z", "src/app/a.rs", 80.0, 100.0),
            cov(1, "2026-06-30T11:00:00Z", "src/app/b.rs", 5.0, 10.0),
        ];
        let rollup = compute_rollup(
            &facts,
            "package",
            Temporal::SemiAdditive,
            &current_capture_ids(&facts, &[]),
        );
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].key, "src/app");
        // (80+5)/(100+10) ≈ 0.7727 — NOT 0.8 + 0.5 = 1.3.
        assert!(
            (rollup[0].value - 85.0 / 110.0).abs() < 1e-9,
            "got {}",
            rollup[0].value
        );
    }

    fn roll(key: &str, value: f64, subject_count: i64) -> RollupRow {
        RollupRow {
            key: key.to_string(),
            value,
            subject_count,
        }
    }

    #[test]
    fn binary_op_parse_word_and_symbol_forms() {
        assert_eq!(BinaryOp::parse("div"), Some(BinaryOp::Div));
        assert_eq!(BinaryOp::parse("/"), Some(BinaryOp::Div));
        assert_eq!(BinaryOp::parse("ratio"), Some(BinaryOp::Div));
        assert_eq!(BinaryOp::parse("+"), Some(BinaryOp::Add));
        assert_eq!(BinaryOp::parse("mul"), Some(BinaryOp::Mul));
        assert_eq!(BinaryOp::parse("pow"), None);
    }

    #[test]
    fn evaluate_formula_inner_joins_on_shared_key() {
        // "bugs per KLOC" by package: bugs / loc, joined on package.
        let bugs = vec![roll("src/app", 10.0, 3), roll("src/util", 4.0, 1)];
        let loc = vec![
            roll("src/app", 2.0, 3),
            roll("src/util", 8.0, 1),
            // src/lib has LOC but no bugs — must NOT appear (inner join, not zero-fill).
            roll("src/lib", 5.0, 2),
        ];
        let derived = evaluate_formula(&bugs, &loc, BinaryOp::Div);
        let rows: Vec<(String, f64, i64)> = derived
            .iter()
            .map(|r| (r.key.clone(), r.value, r.subject_count))
            .collect();
        // app 10/2=5.0 (kept left's subject_count=3), util 4/8=0.5; sorted desc.
        assert_eq!(
            rows,
            vec![
                ("src/app".to_string(), 5.0, 3),
                ("src/util".to_string(), 0.5, 1),
            ]
        );
        assert!(
            !derived.iter().any(|r| r.key == "src/lib"),
            "a key present in only one operand is dropped, not zero-filled"
        );
    }

    #[test]
    fn evaluate_formula_drops_divide_by_zero() {
        let num = vec![roll("a", 3.0, 1), roll("b", 6.0, 1)];
        let den = vec![roll("a", 0.0, 1), roll("b", 2.0, 1)];
        let derived = evaluate_formula(&num, &den, BinaryOp::Div);
        // a: 3/0 is undefined → dropped. b: 6/2 = 3.
        assert_eq!(derived.len(), 1);
        assert_eq!(derived[0].key, "b");
        assert_eq!(derived[0].value, 3.0);
    }

    #[test]
    fn evaluate_formula_supports_other_ops() {
        let a = vec![roll("x", 5.0, 1)];
        let b = vec![roll("x", 3.0, 1)];
        assert_eq!(evaluate_formula(&a, &b, BinaryOp::Add)[0].value, 8.0);
        assert_eq!(evaluate_formula(&a, &b, BinaryOp::Sub)[0].value, 2.0);
        assert_eq!(evaluate_formula(&a, &b, BinaryOp::Mul)[0].value, 15.0);
    }

    #[test]
    fn fact_filter_parses_from_json() {
        let f = FactFilter::from_json("{\"min_value\":10.0}").unwrap();
        assert_eq!(f.min_value, Some(10.0));
        assert!(f.severity.is_none());
        // Partial JSON is fine; a full filter round-trips including the dim pair.
        let f2 = FactFilter::from_json(
            "{\"severity\":\"error\",\"dim_eq\":[\"oxplow.language\",\"rust\"]}",
        )
        .unwrap();
        assert_eq!(f2.severity.as_deref(), Some("error"));
        assert_eq!(
            f2.dim_eq,
            Some(("oxplow.language".to_string(), "rust".to_string()))
        );
        // Malformed → surfaced error, never silently dropped.
        assert!(FactFilter::from_json("{not json").is_err());
    }

    // --- spec-wrapper (DB-backed) -----------------------------------------

    use oxplow_db::{Database, NewFact, NewMetricCapture, NewMetricSpec, SqliteFactStore};

    /// A migrated in-memory store with streams 1 (primary) + 2 (a worktree) so
    /// capture FKs resolve and cross-worktree scoping is exercisable, plus the
    /// engine over it and the seeded `oxplow.complexity` measure id.
    async fn engine_fixture() -> (MetricEngine, SqliteFactStore, i64) {
        use oxplow_domain::stores::StreamStore;
        let db = Database::in_memory();
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        for (id, kind) in [
            (1, oxplow_domain::StreamKind::Primary),
            (2, oxplow_domain::StreamKind::Worktree),
        ] {
            streams
                .upsert(&oxplow_domain::Stream {
                    id: oxplow_domain::StreamId::new(id),
                    kind,
                    title: "t".into(),
                    branch: "main".into(),
                    branch_ref: "refs/heads/main".into(),
                    branch_source: "main".into(),
                    worktree_path: "/r".into(),
                    working_pane: String::new(),
                    talking_pane: String::new(),
                    working_session_id: String::new(),
                    talking_session_id: String::new(),
                    custom_prompt: None,
                    created_at: oxplow_domain::Timestamp::from_unix_ms(0),
                    updated_at: oxplow_domain::Timestamp::from_unix_ms(0),
                    archived_at: None,
                })
                .await
                .unwrap();
        }
        let facts = SqliteFactStore::new(db);
        // A `complete` measure (the NewMeasure default): these tests exercise the
        // COMPLETE-capture engine path — count-over-threshold, headline-is-the-last-
        // capture, zero-fill, stream scoping — where every capture restates the whole
        // population. The real `oxplow.complexity` is `per-path` (V54, tsk41) and is
        // covered by the tree-fold tests below, which set up snapshots.
        let complexity = facts
            .upsert_measure(oxplow_db::NewMeasure::new("acme.complexity", "Complexity"))
            .await
            .unwrap();
        (MetricEngine::new(facts.clone()), facts, complexity)
    }

    fn cap_at(captured_at: &str) -> NewMetricCapture {
        cap_in(1, captured_at)
    }

    fn cap_in(stream: i64, captured_at: &str) -> NewMetricCapture {
        NewMetricCapture {
            captured_at: Some(ts(captured_at)),
            ..NewMetricCapture::done(stream, "metrics", "builtin")
        }
    }

    #[tokio::test]
    async fn clean_analyzer_reads_zero_not_blank() {
        // tsk62: an analyzer that has been clean since day one has ZERO
        // `oxplow.lint_hit` facts, so fact-derived producer discovery finds
        // nothing and its "ran, found nothing" captures could never zero-fill
        // — `oxplow.analysis.*` read blank forever instead of 0. The seed from
        // `source = 'analysis-report'` captures fixes exactly this.
        let (engine, facts, _complexity) = engine_fixture().await;
        let mut cap = NewMetricCapture::done(1, "clippy", "analysis-report");
        cap.captured_at = Some(ts("2026-06-30T10:00:00Z"));
        facts.record_facts(cap, Vec::new()).await.unwrap();

        let mut spec = NewMetricSpec::base("acme.warnings", "Warnings", "oxplow.lint_hit", "count");
        spec.filter_json = Some(r#"{"severity":"warning"}"#.into());
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.warnings").await.unwrap().unwrap();
        assert_eq!(
            engine.headline_for_spec(&spec).await.unwrap(),
            Some(0.0),
            "a clean analyzer run must read 0, not blank"
        );
    }

    #[tokio::test]
    async fn series_for_spec_counts_over_threshold_and_headline_is_last_snapshot() {
        let (engine, facts, complexity) = engine_fixture().await;
        // Capture 1 (10:00): two functions ≥10. Capture 2 (11:00): one ≥10.
        facts
            .record_facts(
                cap_at("2026-06-30T10:00:00Z"),
                vec![
                    NewFact::new(complexity, 14.0),
                    NewFact::new(complexity, 11.0),
                    NewFact::new(complexity, 3.0),
                ],
            )
            .await
            .unwrap();
        facts
            .record_facts(
                cap_at("2026-06-30T11:00:00Z"),
                vec![
                    NewFact::new(complexity, 20.0),
                    NewFact::new(complexity, 2.0),
                ],
            )
            .await
            .unwrap();

        let mut spec = NewMetricSpec::base("acme.hotspots", "Hotspots", "acme.complexity", "count");
        spec.filter_json = Some("{\"min_value\":10.0}".into());
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.hotspots").await.unwrap().unwrap();

        let series = engine.series_for_spec(&spec, None).await.unwrap();
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![2.0, 1.0],
            "one point per capture, time-ascending"
        );
        // complexity is semi-additive → headline is the LAST capture's value.
        assert_eq!(engine.headline_for_spec(&spec).await.unwrap(), Some(1.0));
    }

    #[tokio::test]
    async fn findings_for_spec_projects_offenders_with_derived_severity() {
        let (engine, facts, complexity) = engine_fixture().await;
        facts
            .record_facts(
                cap_at("2026-06-30T10:00:00Z"),
                vec![
                    NewFact {
                        subject_ref: Some("symbol:a::f".into()),
                        path: Some("a.rs".into()),
                        line: Some(3),
                        ..NewFact::new(complexity, 25.0)
                    },
                    NewFact {
                        subject_ref: Some("symbol:a::g".into()),
                        ..NewFact::new(complexity, 12.0)
                    },
                    // Below the filter — not an offender.
                    NewFact::new(complexity, 4.0),
                ],
            )
            .await
            .unwrap();
        let mut spec = NewMetricSpec::base("acme.hot", "Hot", "acme.complexity", "count");
        spec.filter_json = Some("{\"min_value\":10.0}".into());
        spec.direction = "lower-better".into();
        spec.warn_at = Some(15.0);
        spec.fail_at = Some(20.0);
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.hot").await.unwrap().unwrap();

        let findings = engine.findings_for_spec(&spec, None).await.unwrap();
        assert_eq!(findings.len(), 2, "the two facts over the threshold");
        let f = |sref: &str| {
            findings
                .iter()
                .find(|x| x.subject_ref.as_deref() == Some(sref))
                .unwrap()
        };
        // 25 ≥ fail_at 20 (lower-better) → derived "fail"; 12 crosses neither.
        assert_eq!(f("symbol:a::f").severity.as_deref(), Some("fail"));
        assert_eq!(f("symbol:a::f").line, Some(3));
        assert_eq!(f("symbol:a::g").severity, None);
    }

    #[tokio::test]
    async fn series_carries_branch_and_provenance_and_groups_by_pseudo_dims() {
        let (engine, facts, complexity) = engine_fixture().await;
        let mut cap = cap_at("2026-06-30T10:00:00Z");
        cap.branch = Some("feature/x".into());
        facts
            .record_facts(
                cap,
                vec![NewFact {
                    subject_ref: Some("src/a.rs".into()),
                    ..NewFact::new(complexity, 5.0)
                }],
            )
            .await
            .unwrap();
        let spec = NewMetricSpec::base("acme.c", "C", "acme.complexity", "sum");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.c").await.unwrap().unwrap();

        let series = engine.series_for_spec(&spec, None).await.unwrap();
        assert_eq!(series[0].branch.as_deref(), Some("feature/x"));
        assert_eq!(series[0].provenance.as_deref(), Some("observed"));
        // The branch/subject pseudo-dims are groupable server-side.
        let by_branch = engine
            .series_for_spec(&spec, Some("oxplow.branch"))
            .await
            .unwrap();
        assert_eq!(by_branch[0].group.as_deref(), Some("feature/x"));
        let by_subject = engine
            .series_for_spec(&spec, Some("subject"))
            .await
            .unwrap();
        assert_eq!(by_subject[0].group.as_deref(), Some("src/a.rs"));
    }

    #[tokio::test]
    async fn rollup_for_spec_applies_filter_then_groups_by_dimension() {
        let (engine, facts, complexity) = engine_fixture().await;
        facts
            .record_facts(
                cap_at("2026-06-30T10:00:00Z"),
                vec![
                    NewFact {
                        subject_ref: Some("src/app/a.rs".into()),
                        path: Some("src/app/a.rs".into()),
                        ..NewFact::new(complexity, 12.0)
                    },
                    NewFact {
                        // Below the threshold — must be filtered out before rollup.
                        subject_ref: Some("src/app/b.rs".into()),
                        path: Some("src/app/b.rs".into()),
                        ..NewFact::new(complexity, 3.0)
                    },
                    NewFact {
                        subject_ref: Some("src/util/c.rs".into()),
                        path: Some("src/util/c.rs".into()),
                        ..NewFact::new(complexity, 20.0)
                    },
                ],
            )
            .await
            .unwrap();

        let mut spec = NewMetricSpec::base("acme.h", "H", "acme.complexity", "sum");
        spec.filter_json = Some("{\"min_value\":10.0}".into());
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.h").await.unwrap().unwrap();

        let rollup = engine.rollup_for_spec(&spec, "package").await.unwrap();
        let rows: Vec<(String, f64)> = rollup.iter().map(|r| (r.key.clone(), r.value)).collect();
        // b.rs (3.0) filtered out; util (20) sorts above app (12).
        assert_eq!(
            rows,
            vec![
                ("src/util".to_string(), 20.0),
                ("src/app".to_string(), 12.0),
            ]
        );
    }

    #[tokio::test]
    async fn spec_wrapper_returns_empty_for_formula_metric() {
        let (engine, facts, _c) = engine_fixture().await;
        let mut spec = NewMetricSpec::base("acme.derived", "Derived", "", "ratio");
        spec.source_measure = None;
        spec.formula = Some("{\"op\":\"div\",\"left\":\"a\",\"right\":\"b\"}".into());
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.derived").await.unwrap().unwrap();

        assert!(engine
            .series_for_spec(&spec, None)
            .await
            .unwrap()
            .is_empty());
        assert!(engine
            .rollup_for_spec(&spec, "package")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(engine.headline_for_spec(&spec).await.unwrap(), None);
    }

    #[test]
    fn language_dim_reads_both_conformed_and_legacy_bare_keys() {
        // The conformed catalog (V43) declares `oxplow.language`; the bundled
        // gauge scripts emit it namespaced, but facts recorded before the
        // rename carry bare `language` — and the Explorer's declared
        // sliceable_dims still request the bare form. Both request forms must
        // read both fact vintages.
        let namespaced = FactRow {
            dims_json: Some("{\"oxplow.language\":\"rust\"}".into()),
            ..fact(1, "2026-06-30T10:00:00Z", 1.0)
        };
        let legacy = FactRow {
            dims_json: Some("{\"language\":\"rust\"}".into()),
            ..fact(1, "2026-06-30T10:00:00Z", 1.0)
        };
        for f in [&namespaced, &legacy] {
            for key in ["oxplow.language", "language"] {
                assert_eq!(
                    dim_value(f, key).as_deref(),
                    Some("rust"),
                    "dims {:?} requested as {key}",
                    f.dims_json
                );
            }
        }
    }

    #[test]
    fn avg_series_carries_components_so_the_non_additive_collapse_is_the_mean() {
        // Mean-across-closes measures (cycle time, efforts-per-task; V47) are
        // non-additive with `avg` aggregation: capture 1 has one close (100),
        // capture 2 has two (200, 400).
        let facts = vec![
            fact(1, "2026-06-30T10:00:00Z", 100.0),
            fact(2, "2026-06-30T11:00:00Z", 200.0),
            fact(2, "2026-06-30T11:00:00Z", 400.0),
        ];
        let series = aggregate_series(&facts, Aggregation::Avg, &FactFilter::default(), None);
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![100.0, 300.0]
        );
        // Σn/Σd across points must be the mean across ALL closes (700/3) — not
        // 0.0 (a den=0 short-circuit) and not a mean of the capture means (200).
        let collapsed = range_value(&series, Temporal::NonAdditive).unwrap();
        assert!((collapsed - 700.0 / 3.0).abs() < 1e-9, "got {collapsed}");
    }

    #[tokio::test]
    async fn avg_spec_over_non_additive_measure_headlines_the_mean_across_captures() {
        // effort.cycle_time_ms's exact shape: aggregation `avg` over the
        // non-additive oxplow.cycle_time measure, one fact per close carrying
        // numerator=value / denominator=1 (tsk42).
        let (engine, facts, _c) = engine_fixture().await;
        let cycle = facts
            .get_measure("oxplow.cycle_time")
            .await
            .unwrap()
            .expect("migration seeds oxplow.cycle_time")
            .id;
        for (at, v) in [
            ("2026-06-30T10:00:00Z", 100.0),
            ("2026-06-30T11:00:00Z", 200.0),
        ] {
            facts
                .record_facts(
                    NewMetricCapture {
                        captured_at: Some(ts(at)),
                        ..NewMetricCapture::done(1, "effort-lifecycle", "builtin")
                    },
                    vec![NewFact {
                        numerator: Some(v),
                        denominator: Some(1.0),
                        ..NewFact::new(cycle, v)
                    }],
                )
                .await
                .unwrap();
        }
        let spec = NewMetricSpec::base("acme.cycle", "Cycle", "oxplow.cycle_time", "avg");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.cycle").await.unwrap().unwrap();
        assert_eq!(
            engine.headline_for_spec(&spec).await.unwrap(),
            Some(150.0),
            "the mean across closes, not a den=0 collapse to 0"
        );
    }

    #[tokio::test]
    async fn series_and_headline_scoped_to_a_stream_ignore_other_worktrees() {
        let (engine, facts, complexity) = engine_fixture().await;
        // The main worktree scans 40; a feature worktree scans 3 an hour later.
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![NewFact::new(complexity, 40.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(
                cap_in(2, "2026-06-30T11:00:00Z"),
                vec![NewFact::new(complexity, 3.0)],
            )
            .await
            .unwrap();
        let spec = NewMetricSpec::base("acme.total", "Total", "acme.complexity", "sum");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.total").await.unwrap().unwrap();

        // Unscoped read: every worktree's captures (the caller asked for all).
        assert_eq!(engine.series_for_spec(&spec, None).await.unwrap().len(), 2);
        // Stream-scoped: only that worktree's timeline, and the semi-additive
        // headline is ITS last capture — not whichever worktree scanned last.
        let scoped = engine
            .series_for_spec_in_stream(&spec, None, Some(1))
            .await
            .unwrap();
        assert_eq!(
            scoped.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![40.0]
        );
        assert_eq!(
            engine
                .headline_for_spec_in_stream(&spec, Some(1))
                .await
                .unwrap(),
            Some(40.0)
        );
    }

    #[tokio::test]
    async fn the_partial_fold_keeps_each_streams_state_separate() {
        // tsk98: the fold's live state is per WORKTREE. Two worktrees run the
        // same gauge over the same subjects, so they share `(producer, subject)`
        // keys — a state keyed on those alone lets worktree B's run EVICT
        // worktree A's subjects, and the resulting point describes no repo state
        // that ever existed (it's whichever stream wrote last, per subject).
        //
        // The unscoped read is the one that exposes it: it replays every stream's
        // captures into one timeline, so it is where the two states collide.
        let (engine, facts, _c) = engine_fixture().await;
        let test_case = facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new("acme.test_case", "Test case")
            })
            .await
            .unwrap();
        let case = |subject: &str, value: f64| oxplow_db::NewFact {
            subject_ref: Some(subject.into()),
            ..oxplow_db::NewFact::new(test_case, value)
        };
        // Worktree 1 runs both cases; worktree 2 then re-runs only `t1`.
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case("t1", 1.0), case("t2", 1.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(2, "2026-06-30T11:00:00Z"), vec![case("t1", 10.0)])
            .await
            .unwrap();
        let spec = NewMetricSpec::base("acme.cases", "Cases", "acme.test_case", "sum");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.cases").await.unwrap().unwrap();

        // Point 2 is worktree 2's state — `t1` alone. Folding the streams
        // together would yield 11 (worktree 2's t1=10 + worktree 1's stale t2=1),
        // a number belonging to neither worktree.
        assert_eq!(
            engine
                .series_for_spec(&spec, None)
                .await
                .unwrap()
                .iter()
                .map(|p| p.value)
                .collect::<Vec<_>>(),
            vec![2.0, 10.0]
        );
        // Each stream-scoped read sees only its own captures, unchanged.
        for (stream, expected) in [(1, vec![2.0]), (2, vec![10.0])] {
            assert_eq!(
                engine
                    .series_for_spec_in_stream(&spec, None, Some(stream))
                    .await
                    .unwrap()
                    .iter()
                    .map(|p| p.value)
                    .collect::<Vec<_>>(),
                expected,
                "stream {stream}"
            );
        }
    }

    #[tokio::test]
    async fn stream_scoped_zero_fill_skips_other_streams_empty_captures() {
        // A zero-hit scan in ANOTHER worktree must not splice a value-0 point
        // into this stream's series (the tsk44 fill is per the scoped slice).
        let (engine, facts, complexity) = engine_fixture().await;
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![NewFact::new(complexity, 14.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(2, "2026-06-30T11:00:00Z"), vec![])
            .await
            .unwrap();
        let spec = NewMetricSpec::base("acme.count", "Count", "acme.complexity", "count");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.count").await.unwrap().unwrap();

        let unscoped = engine.series_for_spec(&spec, None).await.unwrap();
        assert_eq!(
            unscoped.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![1.0, 0.0],
            "unscoped still zero-fills the other stream's empty scan"
        );
        let scoped = engine
            .series_for_spec_in_stream(&spec, None, Some(1))
            .await
            .unwrap();
        assert_eq!(
            scoped.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![1.0]
        );
    }

    #[tokio::test]
    async fn percent_ratio_spec_reads_on_the_percent_scale() {
        // oxplow.coverage.abs_pct's shape: a `ratio` spec with unit `%` whose
        // facts carry raw line counts (num/den) and a percent value column. The
        // spec-driven reads must come out on the SAME 0..100 scale as the
        // spec's unit + thresholds + per-fact values — not the raw 0..1
        // fraction (a 100× mismatch that rendered 85% coverage as an
        // always-failing "0.85%").
        let (engine, facts, _c) = engine_fixture().await;
        let cov = facts
            .get_measure("oxplow.coverage")
            .await
            .unwrap()
            .expect("migration seeds oxplow.coverage")
            .id;
        facts
            .record_facts(
                cap_at("2026-06-30T10:00:00Z"),
                vec![
                    NewFact {
                        numerator: Some(17.0),
                        denominator: Some(20.0),
                        path: Some("src/a.rs".into()),
                        subject_ref: Some("file:src/a.rs".into()),
                        ..NewFact::new(cov, 85.0)
                    },
                    NewFact {
                        numerator: Some(3.0),
                        denominator: Some(5.0),
                        path: Some("src/b.rs".into()),
                        subject_ref: Some("file:src/b.rs".into()),
                        ..NewFact::new(cov, 60.0)
                    },
                ],
            )
            .await
            .unwrap();
        let mut spec = NewMetricSpec::base("acme.cov", "Coverage", "oxplow.coverage", "ratio");
        spec.unit = Some("%".into());
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.cov").await.unwrap().unwrap();

        // (17+3)/(20+5) = 0.8 → presented as 80.0 (%); the raw components stay
        // on the point for downstream re-aggregation.
        let series = engine.series_for_spec(&spec, None).await.unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].value, 80.0);
        assert_eq!(series[0].numerator, Some(20.0));
        assert_eq!(series[0].denominator, Some(25.0));
        assert_eq!(
            engine.headline_for_spec(&spec).await.unwrap(),
            Some(80.0),
            "headline on the percent scale, comparable to target/warn/fail"
        );
        let rollup = engine.rollup_for_spec(&spec, "package").await.unwrap();
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].key, "src");
        assert_eq!(rollup[0].value, 80.0, "per-group Σn/Σd, percent scale");
    }

    #[tokio::test]
    async fn headline_from_series_matches_headline_for_spec() {
        // The summary read computes the series anyway; collapsing it must equal
        // the from-scratch headline (one fact load instead of two).
        let (engine, facts, complexity) = engine_fixture().await;
        facts
            .record_facts(
                cap_at("2026-06-30T10:00:00Z"),
                vec![NewFact::new(complexity, 5.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(
                cap_at("2026-06-30T11:00:00Z"),
                vec![NewFact::new(complexity, 7.0)],
            )
            .await
            .unwrap();
        let spec = NewMetricSpec::base("acme.sum", "Sum", "acme.complexity", "sum");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.sum").await.unwrap().unwrap();

        let series = engine.series_for_spec(&spec, None).await.unwrap();
        let from_series = engine.headline_from_series(&spec, &series).await.unwrap();
        assert_eq!(from_series, engine.headline_for_spec(&spec).await.unwrap());
        assert_eq!(from_series, Some(7.0), "semi-additive → the last capture");
    }

    #[tokio::test]
    async fn spec_wrapper_errors_on_aggregation_the_engine_cannot_compute() {
        let (engine, facts, _c) = engine_fixture().await;
        // `p95` is a valid schema aggregation but not yet implemented by the engine.
        let spec = NewMetricSpec::base("acme.p", "P", "acme.complexity", "p95");
        facts.upsert_spec(spec).await.unwrap();
        let spec = facts.get_spec("acme.p").await.unwrap().unwrap();

        let err = engine.series_for_spec(&spec, None).await.unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "got {err:?}");
    }
}
