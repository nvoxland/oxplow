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
//! app's database), then times three empty bursts and one burst after a
//! single capture lands on the busiest measure.

// Dev-only measurement tool — `unwrap()` is fine here.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use oxplow_app::metric_cube::MetricCubeBuilder;
use oxplow_db::{Database, NewFact, NewMetricCapture, SqliteFactStore};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: cube_burst <path to a DB COPY>");
    let db = Database::open(&path).expect("open the db copy");
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

    // One capture on the busiest measure, as a recording would add it.
    let (measure_id, stream_id): (i64, i64) = db
        .read(|tx| {
            tx.query_row(
                "SELECT f.measure_id, c.stream_id FROM fact f
                   JOIN metric_capture c ON c.id = f.capture_id
                  GROUP BY f.measure_id ORDER BY count(*) DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    facts
        .record_facts(
            NewMetricCapture::done(stream_id, "cube-burst", "cube-burst"),
            vec![NewFact::new(measure_id, 1.0)],
        )
        .await
        .unwrap();
    // Per measure, so a slow fold is named.
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
    eprintln!(
        "one-capture burst: {folded} folded in {} ms",
        t.elapsed().as_millis()
    );
}
