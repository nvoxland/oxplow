//! Cube-vs-facts equivalence on a REAL database copy (tsk96 / tsk99 / tsk97).
//!
//! For every spec in the catalog: take the fact-served series (the oracle),
//! build the cube, re-read, and demand exact equality. Each base spec is also
//! probed through `cube_series` directly, so a read that silently DECLINED
//! cannot pass vacuously as "equal because both sides were fact-served" —
//! tsk99's trap, and the reason the oracle must be taken BEFORE the build.
//!
//! Run against a consistent copy, never the live file:
//!     sqlite3 .oxplow/local.sqlite "VACUUM INTO '/tmp/cube-eq.sqlite'"
//!     cargo run -p oxplow-app --example cube_equivalence --release -- /tmp/cube-eq.sqlite
//!
//! Opening the copy migrates it — V63 clears the cube, so the oracle pass on a
//! fresh copy is genuinely fact-served. The empty-cube assert below catches a
//! re-used (already built) copy.

// Dev-only verification tool — `unwrap()` is fine here, so relax the
// workspace `unwrap_used` guardrail for this example.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use oxplow_app::metric_cube::{cube_series, MetricCubeBuilder};
use oxplow_app::metric_engine::{parse_capture_scope, spec_aggregation, spec_filter, MetricEngine};
use oxplow_db::{Database, SqliteFactStore};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: cube_equivalence <path to a DB COPY>");
    let db = Database::open(&path).expect("open db copy (migrates it, incl. the V63 cube clear)");
    let facts = SqliteFactStore::new(db);
    let engine = MetricEngine::new(facts.clone());
    let builder = MetricCubeBuilder::new(facts.clone());

    let measures = facts.list_measures().await.unwrap();
    for m in &measures {
        assert!(
            facts
                .cube_rows_for_measure(m.id, None)
                .await
                .unwrap()
                .is_empty(),
            "cube already built for `{}` — the oracle below would just be the cube \
             confirming itself; start from a fresh pre-build copy",
            m.key
        );
    }

    let specs = facts.list_specs().await.unwrap();
    eprintln!("{} specs over {} measures", specs.len(), measures.len());

    // The ORACLE: every spec fact-served, timed.
    let t = Instant::now();
    let mut oracles = Vec::new();
    for s in &specs {
        oracles.push(engine.series_for_spec(s, None).await.unwrap());
    }
    let fact_ms = t.elapsed().as_millis();
    eprintln!("FACT : {} specs in {fact_ms} ms", specs.len());

    let t = Instant::now();
    let folded = builder.build_all().await;
    eprintln!(
        "BUILD: {folded} captures folded in {} ms",
        t.elapsed().as_millis()
    );
    assert!(
        folded > 0,
        "the build folded nothing — nothing was verified"
    );

    // The CUBE pass: same reads, and per-spec, did the cube actually answer?
    let t = Instant::now();
    let mut diverged: Vec<String> = Vec::new();
    let mut served = 0usize;
    let mut declined: Vec<String> = Vec::new();
    for (s, oracle) in specs.iter().zip(&oracles) {
        let read = engine.series_for_spec(s, None).await.unwrap();
        if &read != oracle {
            diverged.push(s.key.clone());
        }
        // Probe the navigation decision for BASE specs (a formula spec reads
        // through its inputs, which are themselves probed here).
        let Some(measure_key) = s.source_measure.as_deref() else {
            continue;
        };
        let Some(measure) = facts.get_measure(measure_key).await.unwrap() else {
            continue;
        };
        let scope = parse_capture_scope(measure_key, &measure.capture_scope).unwrap();
        let agg = spec_aggregation(s).unwrap();
        let filter = spec_filter(s).unwrap();
        match cube_series(&facts, &measure, scope, agg, &filter, None, None)
            .await
            .unwrap()
        {
            Some(_) => served += 1,
            None => declined.push(format!("{} ({})", s.key, s.aggregation)),
        }
    }
    let cube_ms = t.elapsed().as_millis();
    eprintln!("CUBE : {} specs in {cube_ms} ms", specs.len());
    eprintln!(
        "        {served} cube-served, {} declined to facts:",
        declined.len()
    );
    for d in &declined {
        eprintln!("        - {d}");
    }

    assert!(
        diverged.is_empty(),
        "cube-served series DIVERGED from the fact oracle: {diverged:?}"
    );
    eprintln!(
        "OK   : all {} series identical; read time {fact_ms} ms -> {cube_ms} ms",
        specs.len()
    );
}
