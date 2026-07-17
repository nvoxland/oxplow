//! The metric aggregate cube's BUILDER (tsk96) — the write side of the
//! materialized fold.
//!
//! A `per-path` / `per-subject` measure's series is a stateful replay:
//! `state[N] = state[N-1] − restated(N) + facts(N)`, `point[N] = agg(state[N])`.
//! [`metric_engine::tree_state_series`] does that in memory on every read, which
//! is why one sparkline over `oxplow.test_case` decodes 240k facts. This module
//! runs the SAME fold once at WRITE time and stores its output, so a read becomes
//! a GROUP BY over a few hundred cube rows.
//!
//! **It lives in oxplow-app, not oxplow-db, on purpose:** the bucketing needs
//! [`metric_engine::dim_value`] and [`metric_engine::Cell`], and oxplow-db cannot
//! depend on oxplow-app. Doing it in SQL instead would mean a SECOND
//! implementation of dimension extraction, free to drift from the one the read
//! uses. One implementation, called from both sides, is the whole point.
//!
//! Consequently the build runs OUTSIDE `record_facts`' transaction. That is safe
//! because of two properties, and only those two:
//! 1. **Whole-capture idempotence** — applying a capture evicts-then-inserts, and
//!    `write_cube_rows` deletes the capture's rows before writing. Re-running a
//!    capture lands in the same place.
//! 2. **The watermark is advanced last, atomically with the rows.** A torn build
//!    leaves the watermark un-advanced, and an un-advanced watermark reads as
//!    "not cubed yet ⇒ use the facts". Failure degrades to slow, never to wrong.
//!
//! The cube is an ACCELERATOR and is DISPOSABLE — see `.context/metrics.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use oxplow_db::{FactRow, Measure, MetricCapture, NewCubeRow, SqliteFactStore};
use oxplow_domain::{DomainError, Timestamp};

use crate::metric_engine::{
    dim_value, parse_capture_scope, repo_scalar_key, Aggregation, CaptureScope, Cell, FactFilter,
    SeriesPoint, SCALAR_SUBJECT,
};

/// Folds a measure's captures into `metric_cube`, incrementally from the
/// watermark.
#[derive(Clone)]
pub struct MetricCubeBuilder {
    facts: SqliteFactStore,
}

/// The canonical `dims_key` for a bucket: the promoted dimension values a fact
/// carries, as sorted-key JSON. `{}` when nothing is promoted or the fact carries
/// none of the promoted dims.
///
/// **`BTreeMap` is load-bearing, not tidiness** — `dims_key` is part of the cube's
/// PRIMARY KEY, so an unstable field order would write the same logical bucket
/// under two keys and double-count it on merge.
fn dims_key(f: &FactRow, promoted: &[String]) -> String {
    let map: BTreeMap<&str, String> = promoted
        .iter()
        .filter_map(|d| dim_value(f, d).map(|v| (d.as_str(), v)))
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

impl MetricCubeBuilder {
    pub fn new(facts: SqliteFactStore) -> Self {
        Self { facts }
    }

    /// Build (incrementally) the cube for one measure, across every stream.
    /// Returns the number of captures folded.
    ///
    /// A `complete`-scope measure is a NO-OP: its series is a plain per-capture
    /// aggregate, not a fold, so it needs a different build rule and does not
    /// unify with this one (tsk99).
    ///
    /// The caller must treat a failure as non-fatal — an unbuilt cube is a slow
    /// read, not a broken one.
    pub async fn build_measure(&self, measure_key: &str) -> Result<usize, DomainError> {
        let Some(measure) = self.facts.get_measure(measure_key).await? else {
            return Ok(0);
        };
        let scope = parse_capture_scope(measure_key, &measure.capture_scope)?;
        if !scope.is_partial() {
            return Ok(0);
        }
        let producers = self.facts.producers_for_measure(measure.id).await?;
        let captures = self.facts.captures_for_producers(producers).await?;

        // Promoted dims ARE the cube's grain. Read once per build: promoting a dim
        // later changes the grain, which is a cube REBUILD, never a schema change
        // (the raw facts always keep every dim).
        let promoted: Vec<String> = self
            .facts
            .list_dimensions()
            .await?
            .into_iter()
            .filter(|d| d.promoted)
            .map(|d| d.key)
            .collect();

        // One fold per stream — a stream is a WORKTREE and the fold reconstructs
        // one worktree's state (tsk98).
        let mut by_stream: BTreeMap<i64, Vec<MetricCapture>> = BTreeMap::new();
        for c in captures {
            by_stream.entry(c.stream_id).or_default().push(c);
        }
        let mut folded = 0;
        for (stream, caps) in by_stream {
            folded += self
                .build_stream(&measure, scope, stream, &caps, &promoted)
                .await?;
        }
        Ok(folded)
    }

    /// Fold one stream's un-cubed captures. `caps` is that stream's captures
    /// oldest-first (as `captures_for_producers` returns them).
    async fn build_stream(
        &self,
        measure: &Measure,
        scope: CaptureScope,
        stream: i64,
        caps: &[MetricCapture],
        promoted: &[String],
    ) -> Result<usize, DomainError> {
        let watermark = self.facts.cube_watermark(measure.id, stream).await?;
        // Strictly after the watermark, on the same `(captured_at, id)` key the
        // capture list is ordered by.
        //
        // This assumes captures arrive in time order within a (measure, stream) —
        // true because `captured_at` defaults to now() at record time and NO
        // production caller overrides it. A backwards clock jump would leave a
        // capture un-folded and silently absent from a cube-served series; the
        // escape hatch is a rebuild, and the cube is disposable by design.
        let todo: Vec<&MetricCapture> = caps
            .iter()
            .filter(|c| watermark.map_or(true, |w| (c.captured_at, c.id) > w))
            .collect();
        if todo.is_empty() {
            return Ok(0);
        }

        for c in &todo {
            let own = self
                .facts
                .facts_for_captures(measure.id, vec![c.id])
                .await?;
            let restated = self.restated_by(scope, c, &own).await?;
            let inserted: Vec<(String, i64)> = own
                .iter()
                .map(|f| {
                    (
                        repo_scalar_key(f).unwrap_or(SCALAR_SUBJECT).to_string(),
                        f.id,
                    )
                })
                .collect();
            self.facts
                .apply_capture_to_live_state(
                    measure.id,
                    stream,
                    c.producer.clone(),
                    restated,
                    inserted,
                )
                .await?;

            // Re-aggregate the WHOLE live state rather than adjusting the previous
            // row. min/max are not decrementable — evict the subject holding the
            // max and the new max is unrecoverable from the aggregate alone — so
            // delta arithmetic would be silently wrong for `slowest_ms` (a `max`
            // over a per-subject measure). A fresh aggregate is correct for every
            // aggregation by construction.
            let live = self.facts.live_facts(measure.id, stream).await?;
            let rows = cube_rows(&live, promoted);
            self.facts
                .write_cube_rows(measure.id, stream, c.id, c.captured_at, rows)
                .await?;
        }
        Ok(todo.len())
    }

    /// The subject keys a capture RESTATES — the fold's eviction set. This is the
    /// one place the two partial scopes differ.
    async fn restated_by(
        &self,
        scope: CaptureScope,
        c: &MetricCapture,
        own: &[FactRow],
    ) -> Result<Vec<String>, DomainError> {
        let mut keys: Vec<String> = match scope {
            // The capture's snapshot's file rows — NOT the facts it emitted. A
            // file whose count dropped to 0 emits no fact but is still scanned, and
            // a deletion IS a scan result.
            CaptureScope::PerPath => self
                .facts
                .scanned_paths_for_captures(vec![c.id])
                .await?
                .into_iter()
                .map(|(_, path)| path)
                .collect(),
            // A test run restates exactly the cases it executed.
            _ => own
                .iter()
                .filter_map(|f| f.subject_ref.clone())
                .collect::<Vec<_>>(),
        };
        // A capture may also carry PATH-LESS facts (an agent-asserted repo scalar).
        // No path means the fold can't place or supersede them, so they keep the
        // plain "latest assertion per producer wins" rule — emitting one restates
        // it. Mirrors `tree_state_series` exactly.
        if own.iter().any(|f| repo_scalar_key(f).is_none()) {
            keys.push(SCALAR_SUBJECT.to_string());
        }
        Ok(keys)
    }
}

/// The cube-served series, or **`None` when the cube cannot answer this read
/// exactly** — in which case the caller uses the facts, which answer everything.
///
/// This is Kimball aggregate navigation: the query layer picks the smallest table
/// that can answer. `None` is the ordinary, expected outcome for a large class of
/// reads and is never a failure — the cube is a LOSSY projection by construction,
/// and that lossiness is precisely what makes it fast. Reads that stay on the
/// facts permanently, by design:
/// - a **`min_value` threshold** (`oxplow.high_complexity_fns`,
///   `oxplow.long_functions`) — the cube summed the individual values away;
/// - **`group_by` on an unpromoted dim** (`subject` would need a bucket per test,
///   i.e. the fact table again);
/// - **`last`**, which is not decomposable;
/// - anything the cube has not caught up with (see the watermark below).
///
/// Correctness rule: **never let this return data the facts don't have.** Every
/// branch here either reproduces `tree_state_series` exactly or bails to `None`.
#[allow(clippy::too_many_arguments)]
pub async fn cube_series(
    facts: &SqliteFactStore,
    measure: &Measure,
    scope: CaptureScope,
    agg: Aggregation,
    filter: &FactFilter,
    group_by: Option<&str>,
    stream: Option<i64>,
) -> Result<Option<Vec<SeriesPoint>>, DomainError> {
    // --- eligibility: can the cube answer this EXACTLY? ---
    // A complete-scope measure's series is a plain per-capture aggregate, not a
    // fold — a different build rule that does not unify with this one (tsk99).
    if !scope.is_partial() || !Cell::decomposes(agg) || filter.min_value.is_some() {
        return Ok(None);
    }
    let promoted: BTreeSet<String> = facts
        .list_dimensions()
        .await?
        .into_iter()
        .filter(|d| d.promoted)
        .map(|d| d.key)
        .collect();
    // Every dim the read filters or slices on must be IN the grain — the cube
    // knows nothing about a dim it didn't bucket by.
    let needed = filter
        .severity
        .as_ref()
        .map(|_| "oxplow.severity")
        .into_iter()
        .chain(filter.dim_eq.iter().map(|(k, _)| k.as_str()))
        .chain(group_by);
    if !needed.into_iter().all(|d| promoted.contains(d)) {
        return Ok(None);
    }

    // The capture list, built exactly as the fact path builds it — but from
    // CAPTURES, never from facts. Deriving it by scanning facts is the 374k-row
    // decode the cube exists to remove, so doing it here would fix nothing.
    let producers = facts.producers_for_measure(measure.id).await?;
    if producers.is_empty() {
        // Leave the empty-producer case to the facts: `oxplow.lint_hit` seeds its
        // producers from `analysis-report` captures there (tsk62), and this path
        // must never be the reason a metric reads blank.
        return Ok(None);
    }
    let captures: Vec<MetricCapture> = facts
        .captures_for_producers(producers)
        .await?
        .into_iter()
        .filter(|c| stream.map_or(true, |s| c.stream_id == s))
        .collect();
    if captures.is_empty() {
        return Ok(None);
    }

    // The cube must cover EVERY capture this read would use. "No cube rows for
    // capture N" is otherwise AMBIGUOUS — legitimately-empty state (a real 0
    // point) vs not-cubed-yet — and conflating those is how a materialized read
    // reports 0 instead of admitting it doesn't know.
    let mut watermarks: HashMap<i64, (Timestamp, i64)> = HashMap::new();
    for stream_id in captures
        .iter()
        .map(|c| c.stream_id)
        .collect::<BTreeSet<i64>>()
    {
        let Some(w) = facts.cube_watermark(measure.id, stream_id).await? else {
            return Ok(None);
        };
        watermarks.insert(stream_id, w);
    }
    if !captures.iter().all(|c| {
        watermarks
            .get(&c.stream_id)
            .is_some_and(|w| (c.captured_at, c.id) <= *w)
    }) {
        return Ok(None);
    }

    // --- serve ---
    let rows = facts.cube_rows_for_measure(measure.id, stream).await?;
    let mut kept: Vec<(&oxplow_db::CubeReadRow, Option<String>)> = Vec::new();
    for r in &rows {
        let dims: BTreeMap<String, String> = serde_json::from_str(&r.dims_key).unwrap_or_default();
        let hit = |key: &str, want: &str| dims.get(key).map(String::as_str) == Some(want);
        if let Some(sev) = &filter.severity {
            if !hit("oxplow.severity", sev) {
                continue;
            }
        }
        if let Some((key, val)) = &filter.dim_eq {
            if !hit(key, val) {
                continue;
            }
        }
        let group = match group_by {
            // A bucket missing the slice dim can't be placed on the axis —
            // `tree_state_series` drops it, so this must too.
            Some(dim) => match dims.get(dim) {
                Some(g) => Some(g.clone()),
                None => continue,
            },
            // Ungrouped merges ALL matching buckets, the null-dim one included.
            None => None,
        };
        kept.push((r, group));
    }

    // The producers the FILTER narrows to — the fact path derives these from
    // filter-matching facts, and a fact is live at its own capture, so "ever
    // emitted a matching fact" ≡ "ever had a matching bucket".
    let narrowed: BTreeSet<&str> = kept.iter().map(|(r, _)| r.producer.as_str()).collect();
    if narrowed.is_empty() {
        return Ok(None);
    }

    let mut by_capture: HashMap<i64, BTreeMap<Option<String>, Cell>> = HashMap::new();
    for (r, group) in &kept {
        by_capture
            .entry(r.capture_id)
            .or_default()
            .entry(group.clone())
            .or_default()
            .merge(&Cell {
                count: r.fact_count,
                sum: r.value_sum,
                min: r.value_min,
                max: r.value_max,
                num: r.numerator,
                den: r.denominator,
            });
    }

    let mut out: Vec<SeriesPoint> = Vec::new();
    for c in captures
        .iter()
        .filter(|c| narrowed.contains(c.producer.as_str()))
    {
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
        let groups = by_capture.get(&c.id);
        match group_by {
            None => {
                let cell = groups
                    .and_then(|g| g.get(&None))
                    .copied()
                    .unwrap_or_default();
                // Empty live state ⇒ the fold emits an explicit 0 point (NOT
                // `project`, whose avg would be NaN and max −∞ on an empty cell).
                let triple = if cell.count == 0 {
                    (0.0, None, None)
                } else {
                    let Some(t) = cell.project(agg) else {
                        return Ok(None);
                    };
                    t
                };
                out.push(point(triple.0, triple.1, triple.2, None));
            }
            // Grouped: BTreeMap order, matching `tree_state_series`' sorted groups.
            Some(_) => {
                for (group, cell) in groups.into_iter().flatten() {
                    let Some(t) = cell.project(agg) else {
                        return Ok(None);
                    };
                    out.push(point(t.0, t.1, t.2, group.clone()));
                }
            }
        }
    }
    Ok(Some(out))
}

/// Bucket a live state into its cube rows: `(the fact's producer, dims_key)` →
/// [`Cell`].
///
/// Keyed on the FACT's producer, not the capture's — the state at capture N holds
/// every producer's live facts, including ones whose last run was long ago.
fn cube_rows(live: &[FactRow], promoted: &[String]) -> Vec<NewCubeRow> {
    let mut buckets: BTreeMap<(String, String), Vec<&FactRow>> = BTreeMap::new();
    for f in live {
        buckets
            .entry((f.producer.clone(), dims_key(f, promoted)))
            .or_default()
            .push(f);
    }
    buckets
        .into_iter()
        .map(|((producer, dims_key), fs)| {
            let cell = Cell::of(&fs);
            NewCubeRow {
                producer,
                dims_key,
                fact_count: cell.count,
                value_sum: cell.sum,
                value_min: cell.min,
                value_max: cell.max,
                numerator: cell.num,
                denominator: cell.den,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric_engine::MetricEngine;
    use oxplow_db::{
        Database, MetricSpec, NewFact, NewMetricCapture, NewMetricSpec, SqliteFactStore,
    };

    fn ts(s: &str) -> Timestamp {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    /// A migrated in-memory store with streams 1 + 2 so capture FKs resolve.
    async fn fixture() -> (MetricEngine, SqliteFactStore, MetricCubeBuilder) {
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
                    created_at: Timestamp::from_unix_ms(0),
                    updated_at: Timestamp::from_unix_ms(0),
                    archived_at: None,
                })
                .await
                .unwrap();
        }
        let facts = SqliteFactStore::new(db);
        (
            MetricEngine::new(facts.clone()),
            facts.clone(),
            MetricCubeBuilder::new(facts),
        )
    }

    fn cap_in(stream: i64, captured_at: &str) -> NewMetricCapture {
        NewMetricCapture {
            captured_at: Some(ts(captured_at)),
            ..NewMetricCapture::done(stream, "metrics", "builtin")
        }
    }

    /// The FACT-served series for a spec — the ORACLE.
    ///
    /// Call it **before** the cube is built. Once the cube can answer a read,
    /// `series_for_spec` reads the cube, so an "oracle" taken after a build is
    /// just the cube confirming itself.
    async fn oracle(engine: &MetricEngine, spec: &MetricSpec) -> Vec<SeriesPoint> {
        engine.series_for_spec(spec, None).await.unwrap()
    }

    /// Assert the cube answers `spec` exactly as the facts did — and that it
    /// ANSWERED AT ALL.
    ///
    /// That second half is the point: `cube_series` returns `None` for every read
    /// it can't serve, so without the `expect` a regression that silently disabled
    /// the cube would leave every equality below passing vacuously, green and
    /// meaningless. Specs here must be unscaled — `spec_value_scale` is applied by
    /// `series_for_spec` above this layer, not by `cube_series`.
    async fn assert_cube_answers(
        engine: &MetricEngine,
        facts: &SqliteFactStore,
        measure_key: &str,
        spec: &MetricSpec,
        oracle: &[SeriesPoint],
    ) {
        let measure = facts.get_measure(measure_key).await.unwrap().unwrap();
        let scope = parse_capture_scope(measure_key, &measure.capture_scope).unwrap();
        let agg = crate::metric_engine::spec_aggregation(spec).unwrap();
        let filter = crate::metric_engine::spec_filter(spec).unwrap();
        let served = cube_series(facts, &measure, scope, agg, &filter, None, None)
            .await
            .unwrap()
            .expect("the cube must ANSWER this read — a None fall-through passes vacuously");
        assert_eq!(&served, oracle, "cube-served must equal fact-served");
        assert_eq!(
            &engine.series_for_spec(spec, None).await.unwrap(),
            oracle,
            "the wired read must reach the cube and land in the same place"
        );
    }

    async fn spec(facts: &SqliteFactStore, key: &str, measure: &str, agg: &str) -> MetricSpec {
        facts
            .upsert_spec(NewMetricSpec::base(key, key, measure, agg))
            .await
            .unwrap();
        facts.get_spec(key).await.unwrap().unwrap()
    }

    async fn per_subject_measure(facts: &SqliteFactStore, key: &str) -> i64 {
        facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new(key, key)
            })
            .await
            .unwrap()
    }

    fn case(measure: i64, subject: &str, value: f64) -> NewFact {
        NewFact {
            subject_ref: Some(subject.into()),
            ..NewFact::new(measure, value)
        }
    }

    #[tokio::test]
    async fn the_cube_serves_the_partial_folds_series_exactly() {
        // The 95% case: 119 of 125 real captures are PARTIAL, so this is what the
        // cube must get right. Capture 2 re-runs only `t1`, leaving `t2` live from
        // capture 1 — the fold's whole reason to exist. A cube built from each
        // capture's OWN facts (rather than the live state) reads 10, not 11.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.test_case").await;
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case(m, "t1", 1.0), case(m, "t2", 1.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case(m, "t1", 10.0)])
            .await
            .unwrap();
        let spec = spec(&facts, "acme.cases", "acme.test_case", "sum").await;

        let oracle = oracle(&engine, &spec).await;
        assert_eq!(
            oracle.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![2.0, 11.0],
            "the fold, from the facts"
        );
        assert_eq!(builder.build_measure("acme.test_case").await.unwrap(), 2);
        assert_cube_answers(&engine, &facts, "acme.test_case", &spec, &oracle).await;
    }

    #[tokio::test]
    async fn a_rebuild_folds_only_what_the_watermark_has_not_seen() {
        // Incrementality is the whole economy: a build must cost the NEW capture's
        // facts, not all of history. It must also be safe to re-run — the wiring
        // calls it after every recording, and a torn build re-runs the capture.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.test_case").await;
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case(m, "t1", 1.0), case(m, "t2", 1.0)],
            )
            .await
            .unwrap();
        assert_eq!(builder.build_measure("acme.test_case").await.unwrap(), 1);
        assert_eq!(
            builder.build_measure("acme.test_case").await.unwrap(),
            0,
            "nothing new ⇒ the watermark makes the build a no-op"
        );

        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case(m, "t1", 10.0)])
            .await
            .unwrap();
        assert_eq!(
            builder.build_measure("acme.test_case").await.unwrap(),
            1,
            "only the new capture is folded — not the history before it"
        );
        // Re-running the whole build must not double-insert into the live state.
        builder.build_measure("acme.test_case").await.unwrap();
        let spec = spec(&facts, "acme.cases", "acme.test_case", "sum").await;
        assert_eq!(
            engine
                .series_for_spec(&spec, None)
                .await
                .unwrap()
                .iter()
                .map(|p| p.value)
                .collect::<Vec<_>>(),
            vec![2.0, 11.0],
            "an incremental build lands where a from-scratch one does"
        );
    }

    #[tokio::test]
    async fn an_uncubed_capture_sends_the_read_back_to_the_facts() {
        // The watermark's real job. Without it, "no cube rows for capture N" is
        // AMBIGUOUS — legitimately-empty state (a real 0 point) vs not-cubed-yet —
        // and a cube that guessed would drop the newest point off every sparkline
        // in the window between a test run and its build.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.test_case").await;
        facts
            .record_facts(cap_in(1, "2026-06-30T10:00:00Z"), vec![case(m, "t1", 1.0)])
            .await
            .unwrap();
        builder.build_measure("acme.test_case").await.unwrap();
        // A capture lands but is NOT yet folded.
        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case(m, "t1", 5.0)])
            .await
            .unwrap();

        let measure = facts.get_measure("acme.test_case").await.unwrap().unwrap();
        let filter = FactFilter::from_json("{}").unwrap();
        assert!(
            cube_series(
                &facts,
                &measure,
                CaptureScope::PerSubject,
                Aggregation::Sum,
                &filter,
                None,
                None
            )
            .await
            .unwrap()
            .is_none(),
            "an un-advanced watermark must decline, not serve a stale series"
        );
        // The facts still answer — completely.
        let spec = spec(&facts, "acme.cases", "acme.test_case", "sum").await;
        assert_eq!(
            engine
                .series_for_spec(&spec, None)
                .await
                .unwrap()
                .iter()
                .map(|p| p.value)
                .collect::<Vec<_>>(),
            vec![1.0, 5.0],
            "falling back is a SLOW read, never a wrong or short one"
        );
    }

    #[tokio::test]
    async fn a_value_threshold_spec_never_touches_the_cube() {
        // `oxplow.high_complexity_fns` (min_value 11) and `oxplow.long_functions`
        // (min_value 61) filter on each fact's OWN value — which the cube summed
        // away. These reads stay on the facts permanently, by design; the cube is
        // a lossy projection and the lossiness IS the speedup.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.complexity").await;
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case(m, "a", 14.0), case(m, "b", 3.0)],
            )
            .await
            .unwrap();
        builder.build_measure("acme.complexity").await.unwrap();

        let measure = facts.get_measure("acme.complexity").await.unwrap().unwrap();
        let filter = FactFilter::from_json(r#"{"min_value":11.0}"#).unwrap();
        assert!(
            cube_series(
                &facts,
                &measure,
                CaptureScope::PerSubject,
                Aggregation::Count,
                &filter,
                None,
                None
            )
            .await
            .unwrap()
            .is_none(),
            "the cube cannot threshold values it summed away — it must decline"
        );
        let mut s = NewMetricSpec::base("acme.hot", "Hot", "acme.complexity", "count");
        s.filter_json = Some(r#"{"min_value":11.0}"#.into());
        facts.upsert_spec(s).await.unwrap();
        let s = facts.get_spec("acme.hot").await.unwrap().unwrap();
        assert_eq!(
            engine
                .series_for_spec(&s, None)
                .await
                .unwrap()
                .iter()
                .map(|p| p.value)
                .collect::<Vec<_>>(),
            vec![1.0],
            "the facts answer it, exactly as before the cube existed"
        );
    }

    #[tokio::test]
    async fn the_cube_tracks_max_across_an_eviction_that_removes_the_maximum() {
        // THE test for why `metric_live_fact` exists. `state[N] = state[N-1] −
        // restated + facts` decrements fine for count/sum, but min/max do NOT:
        // evict the subject holding the max and the new max is unrecoverable from
        // the aggregate alone. `oxplow.tests.slowest_ms` is exactly this shape (a
        // `max` over a per-subject measure), so a delta-maintained cube would have
        // been SILENTLY WRONG for it — it would still read 100 here.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.test_duration").await;
        // `t1` is the slowest test, then gets fast. The new max is `t2`'s 5 — a
        // value the previous cube row never carried.
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case(m, "t1", 100.0), case(m, "t2", 5.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case(m, "t1", 1.0)])
            .await
            .unwrap();
        let spec = spec(&facts, "acme.slowest", "acme.test_duration", "max").await;

        let oracle = oracle(&engine, &spec).await;
        assert_eq!(
            oracle.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![100.0, 5.0],
            "the fold, from the facts"
        );
        builder.build_measure("acme.test_duration").await.unwrap();
        assert_cube_answers(&engine, &facts, "acme.test_duration", &spec, &oracle).await;
    }

    #[tokio::test]
    async fn the_cube_keeps_each_streams_state_separate() {
        // The cube-side of tsk98. Two worktrees run the same gauge over the same
        // subjects, so they share `(producer, subject)` keys — a stream-blind live
        // state lets worktree 2's run evict worktree 1's subjects, and the point
        // describes a repo state that never existed. `metric_live_fact` keys by
        // stream for exactly this reason. The UNSCOPED read is where they collide.
        let (engine, facts, builder) = fixture().await;
        let m = per_subject_measure(&facts, "acme.test_case").await;
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case(m, "t1", 1.0), case(m, "t2", 1.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(2, "2026-06-30T11:00:00Z"), vec![case(m, "t1", 10.0)])
            .await
            .unwrap();
        let spec = spec(&facts, "acme.cases", "acme.test_case", "sum").await;

        let oracle = oracle(&engine, &spec).await;
        // Point 2 is worktree 2's state — `t1` alone. A stream-blind state would
        // yield 11 (worktree 2's t1=10 + worktree 1's stale t2=1).
        assert_eq!(
            oracle.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![2.0, 10.0],
            "the fold, from the facts"
        );
        builder.build_measure("acme.test_case").await.unwrap();
        assert_cube_answers(&engine, &facts, "acme.test_case", &spec, &oracle).await;
    }
}
