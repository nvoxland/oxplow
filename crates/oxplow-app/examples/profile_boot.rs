//! Measure what one test's boot costs: `Services::in_memory` over a fresh
//! repository, as `test_fixtures::services_with_effort` starts it.
//!
//! Run from the repo root, in the profile the tests build in:
//!     cargo run -q -p oxplow-app --example profile_boot -- [boots]
//!
//! One line per step, with the median and the slowest over the boots. The
//! first steps are parts of the boot, timed by themselves. The first boot reads
//! the shipped folders, as each test's process does: that is the `slowest`.

// Dev-only measuring tool — `unwrap()` is fine here, so relax the
// workspace `unwrap_used` guardrail for this example.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use oxplow_app::bundled_extensions::BUNDLED;
use oxplow_app::extension::extension_store::{extract_shipped, shipped_hash};
use oxplow_app::extension::extensions::load_extensions;

fn line(step: &str, mut times: Vec<Duration>) {
    times.sort();
    println!(
        "{step:<20} median {:>7.1} ms   slowest {:>7.1} ms",
        times[times.len() / 2].as_secs_f64() * 1000.0,
        times[times.len() - 1].as_secs_f64() * 1000.0,
    );
}

fn timed<T>(times: &mut Vec<Duration>, run: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = run();
    times.push(start.elapsed());
    out
}

#[tokio::main]
async fn main() {
    let boots: usize = std::env::args()
        .nth(1)
        .map_or(20, |n| n.parse().expect("a number of boots"));
    let (mut hash, mut store, mut load, mut boot, mut drop_) =
        (vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..boots {
        let dir = tempfile::tempdir().unwrap();
        timed(&mut hash, || {
            for b in BUNDLED {
                shipped_hash(b);
            }
        });
        let scratch = tempfile::tempdir().unwrap();
        timed(&mut store, || {
            for b in BUNDLED {
                extract_shipped(scratch.path(), b).unwrap();
            }
        });
        timed(&mut load, || load_extensions(scratch.path()));
        oxplow_app::vcs::GitProvider
            .init_repository(dir.path())
            .await
            .unwrap();
        let start = Instant::now();
        let svc = oxplow_app::Services::in_memory(dir.path()).await.unwrap();
        boot.push(start.elapsed());
        // Its tasks keep running: what a test leaves until its process ends.
        std::mem::forget(svc);
        timed(&mut drop_, || drop(dir));
    }
    line("shipped_hash", hash);
    line("store copy", store);
    line("load_extensions", load);
    line("Services::in_memory", boot);
    line("remove temp dir", drop_);
}
