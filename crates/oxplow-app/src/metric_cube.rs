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

use std::collections::BTreeMap;

use oxplow_db::{FactRow, Measure, MetricCapture, NewCubeRow, SqliteFactStore};
use oxplow_domain::DomainError;

use crate::metric_engine::{
    dim_value, parse_capture_scope, repo_scalar_key, CaptureScope, Cell, SCALAR_SUBJECT,
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
    use crate::metric_engine::{Aggregation, MetricEngine};
    use oxplow_db::{Database, NewFact, NewMetricCapture, NewMetricSpec, SqliteFactStore};
    use oxplow_domain::Timestamp;
    use std::collections::HashMap;

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

    /// Re-derive a series from the cube the way the read path will: merge every
    /// bucket of a capture, then project. This is the read's arithmetic, isolated
    /// so the builder can be gated before the read path exists.
    async fn cube_series(facts: &SqliteFactStore, measure_id: i64, agg: Aggregation) -> Vec<f64> {
        let rows = facts.cube_rows_for_measure(measure_id, None).await.unwrap();
        let mut order: Vec<i64> = Vec::new();
        let mut merged: HashMap<i64, Cell> = HashMap::new();
        for r in &rows {
            if !merged.contains_key(&r.capture_id) {
                order.push(r.capture_id);
            }
            merged.entry(r.capture_id).or_default().merge(&Cell {
                count: r.fact_count,
                sum: r.value_sum,
                min: r.value_min,
                max: r.value_max,
                num: r.numerator,
                den: r.denominator,
            });
        }
        order
            .into_iter()
            .map(|id| merged[&id].project(agg).unwrap().0)
            .collect()
    }

    #[tokio::test]
    async fn the_built_cube_reproduces_the_partial_folds_series() {
        // The 95% case: 119 of 125 real captures are PARTIAL, so this is what the
        // cube must get right. Capture 2 re-runs only `t1`, leaving `t2` live from
        // capture 1 — the fold's whole reason to exist. A cube built per-capture
        // (rather than per live state) would read 10, not 11.
        let (engine, facts, builder) = fixture().await;
        let test_case = facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new("acme.test_case", "Test case")
            })
            .await
            .unwrap();
        let case = |subject: &str, value: f64| NewFact {
            subject_ref: Some(subject.into()),
            ..NewFact::new(test_case, value)
        };
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case("t1", 1.0), case("t2", 1.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case("t1", 10.0)])
            .await
            .unwrap();

        assert_eq!(builder.build_measure("acme.test_case").await.unwrap(), 2);

        // The fact path is the ORACLE — the cube's license to exist is agreeing
        // with it, never the other way round.
        facts
            .upsert_spec(NewMetricSpec::base(
                "acme.cases",
                "Cases",
                "acme.test_case",
                "sum",
            ))
            .await
            .unwrap();
        let spec = facts.get_spec("acme.cases").await.unwrap().unwrap();
        let oracle: Vec<f64> = engine
            .series_for_spec(&spec, None)
            .await
            .unwrap()
            .iter()
            .map(|p| p.value)
            .collect();
        assert_eq!(oracle, vec![2.0, 11.0], "the fold, from the facts");
        assert_eq!(
            cube_series(&facts, test_case, Aggregation::Sum).await,
            oracle,
            "the cube must reproduce the fold point for point"
        );
    }

    #[tokio::test]
    async fn a_rebuild_folds_only_what_the_watermark_has_not_seen() {
        // Incrementality is the whole economy: a build must cost the NEW capture's
        // facts, not all of history. It must also be safe to re-run — the wiring
        // calls it after every recording, and a torn build re-runs the capture.
        let (_engine, facts, builder) = fixture().await;
        let m = facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new("acme.test_case", "Test case")
            })
            .await
            .unwrap();
        let case = |subject: &str, value: f64| NewFact {
            subject_ref: Some(subject.into()),
            ..NewFact::new(m, value)
        };
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case("t1", 1.0), case("t2", 1.0)],
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
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case("t1", 10.0)])
            .await
            .unwrap();
        assert_eq!(
            builder.build_measure("acme.test_case").await.unwrap(),
            1,
            "only the new capture is folded — not the history before it"
        );
        // Re-running the whole build must not double-insert into the live state.
        builder.build_measure("acme.test_case").await.unwrap();
        assert_eq!(
            cube_series(&facts, m, Aggregation::Sum).await,
            vec![2.0, 11.0],
            "an incremental build lands where a from-scratch one does"
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
        let m = facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new("acme.test_duration", "Duration")
            })
            .await
            .unwrap();
        let case = |subject: &str, value: f64| NewFact {
            subject_ref: Some(subject.into()),
            ..NewFact::new(m, value)
        };
        // `t1` is the slowest test, then gets fast. The new max is `t2`'s 5 —
        // a value the previous cube row never carried.
        facts
            .record_facts(
                cap_in(1, "2026-06-30T10:00:00Z"),
                vec![case("t1", 100.0), case("t2", 5.0)],
            )
            .await
            .unwrap();
        facts
            .record_facts(cap_in(1, "2026-06-30T11:00:00Z"), vec![case("t1", 1.0)])
            .await
            .unwrap();
        builder.build_measure("acme.test_duration").await.unwrap();

        facts
            .upsert_spec(NewMetricSpec::base(
                "acme.slowest",
                "Slowest",
                "acme.test_duration",
                "max",
            ))
            .await
            .unwrap();
        let spec = facts.get_spec("acme.slowest").await.unwrap().unwrap();
        let oracle: Vec<f64> = engine
            .series_for_spec(&spec, None)
            .await
            .unwrap()
            .iter()
            .map(|p| p.value)
            .collect();
        assert_eq!(oracle, vec![100.0, 5.0], "the fold, from the facts");
        assert_eq!(
            cube_series(&facts, m, Aggregation::Max).await,
            oracle,
            "re-aggregating live state recovers the new max; delta arithmetic could not"
        );
    }

    #[tokio::test]
    async fn the_cube_keeps_each_streams_state_separate() {
        // The cube-side of tsk98. Two worktrees run the same gauge over the same
        // subjects, so they share `(producer, subject)` keys — a stream-blind live
        // state lets worktree 2's run evict worktree 1's subjects, and the point
        // describes a repo state that never existed. `metric_live_fact` keys by
        // stream for exactly this reason.
        let (engine, facts, builder) = fixture().await;
        let m = facts
            .upsert_measure(oxplow_db::NewMeasure {
                capture_scope: "per-subject".into(),
                ..oxplow_db::NewMeasure::new("acme.test_case", "Test case")
            })
            .await
            .unwrap();
        let case = |subject: &str, value: f64| NewFact {
            subject_ref: Some(subject.into()),
            ..NewFact::new(m, value)
        };
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
        builder.build_measure("acme.test_case").await.unwrap();

        facts
            .upsert_spec(NewMetricSpec::base(
                "acme.cases",
                "Cases",
                "acme.test_case",
                "sum",
            ))
            .await
            .unwrap();
        let spec = facts.get_spec("acme.cases").await.unwrap().unwrap();
        let oracle: Vec<f64> = engine
            .series_for_spec(&spec, None)
            .await
            .unwrap()
            .iter()
            .map(|p| p.value)
            .collect();
        // Point 2 is worktree 2's state — `t1` alone. A stream-blind state would
        // yield 11 (worktree 2's t1=10 + worktree 1's stale t2=1).
        assert_eq!(oracle, vec![2.0, 10.0], "the fold, from the facts");
        assert_eq!(cube_series(&facts, m, Aggregation::Sum).await, oracle);
    }
}
