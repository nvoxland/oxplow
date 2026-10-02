//! What the cube asset costs per burst on a REAL database copy (P7.B1):
//! the asset runner calls `build_all` once after each quiet burst of
//! commits to `metric_capture` / `fact`, so its incremental cost — when
//! nothing new landed, and when one capture did — is what every burst
//! pays.
//!
//! Run against a consistent copy, never the live file:
//!     sqlite3 .oxplow/local.sqlite "VACUUM INTO '/tmp/cube-burst.sqlite'"
//!     cargo run -p oxplow-app --example cube_burst --release -- /tmp/cube-burst.sqlite
//!
//! It catches the copy's cube up first (a no-op on a copy of a running
//! app's database), then times three empty bursts, a burst after a test
//! run's capture on the latest run's branch (an incremental fold — what
//! every `test:collect` costs), and the same run on a new branch (that
//! branch's seed).

// Dev-only measurement tool — `unwrap()` is fine here.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use oxplow_app::metric_cube::MetricCubeBuilder;
use oxplow_db::{Database, NewFact, NewMetricCapture, SqliteFactStore};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: cube_burst <path to a DB COPY>");
    let db = Database::open(path).expect("open the db copy");
    let facts = SqliteFactStore::new(db.clone());
    let builder = MetricCubeBuilder::new(facts.clone());

    let t = Instant::now();
    let folded = builder.build_all().await;
    eprintln!(
        "catch-up: {folded} captures folded in {} ms",
        t.elapsed().as_millis()
    );

    for i in 1..=3 {
        let t = Instant::now();
        let folded = builder.build_all().await;
        eprintln!(
            "empty burst {i}: {folded} folded in {} ms",
            t.elapsed().as_millis()
        );
    }

    // What a test run adds: a copy of the latest `tests` capture's
    // per-case facts, on the same branch — an incremental fold into an
    // already-seeded partition, which is what every `test:collect` pays.
    let (latest, branch): (i64, Option<String>) = db
        .read(|tx| {
            tx.query_row(
                "SELECT id, branch FROM metric_capture WHERE producer = 'tests'
                  ORDER BY captured_at DESC, id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    let mut copied = Vec::new();
    for m in facts.list_measures().await.unwrap() {
        for f in facts.facts_for_captures(m.id, vec![latest]).await.unwrap() {
            copied.push(NewFact {
                subject_kind: f.subject_kind.clone(),
                subject_ref: f.subject_ref.clone(),
                path: f.path.clone(),
                dims_json: f.dims_json.clone(),
                ..NewFact::new(m.id, f.value)
            });
        }
    }
    let run = |branch: Option<String>| NewMetricCapture {
        branch,
        ..NewMetricCapture::done(1, "tests", "cube-burst")
    };
    eprintln!(
        "a test run: {} facts on branch {:?}",
        copied.len(),
        branch.as_deref().unwrap_or("")
    );
    facts
        .record_facts(run(branch), copied.clone())
        .await
        .unwrap();
    burst(&facts, &builder, "test-run burst (seeded branch)").await;

    // The same run on a branch the cube has never seen: its first fold
    // seeds the partition from the history visible to it.
    facts
        .record_facts(run(Some("cube-burst-new-branch".into())), copied)
        .await
        .unwrap();
    burst(&facts, &builder, "test-run burst (new branch: a seed)").await;
}

/// One burst, timed per measure so a slow fold is named.
async fn burst(facts: &SqliteFactStore, builder: &MetricCubeBuilder, label: &str) {
    let t = Instant::now();
    let mut folded = 0;
    for m in facts.list_measures().await.unwrap() {
        let one = Instant::now();
        let n = builder.build_measure(&m.key).await.unwrap();
        folded += n;
        let ms = one.elapsed().as_millis();
        if ms >= 50 {
            eprintln!("  {}: {n} folded in {ms} ms", m.key);
        }
    }
    eprintln!("{label}: {folded} folded in {} ms", t.elapsed().as_millis());
}
