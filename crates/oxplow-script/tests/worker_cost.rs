//! What running scripts in worker processes costs, on realistic inputs:
//! a cold trivial run, the whole-workspace lcov parse (~5 MB / 100k lines)
//! and a sweep of the built-in fact metrics over a 120-file tree. It
//! prints the numbers `.context/performance.md` records and asserts only
//! that every run succeeded. Ignored by default (it takes seconds); run it
//! with `bun run test:fast -p oxplow-script --run-ignored only worker_cost`.

mod fixtures;

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::run::Budget;
use oxplow_script::workers::{Limits, RunHost, ScriptRun, ScriptWorkers};
use serde_json::json;

/// Workers with room enough that the measurement measures cost, not the
/// cap (the generated tree is duplicate-dense on purpose).
fn workers() -> ScriptWorkers {
    ScriptWorkers::new(
        env!("CARGO_BIN_EXE_oxplow-script-worker").into(),
        Limits {
            max_concurrent: 1,
            memory_bytes: 8 * 1024 * 1024 * 1024,
        },
    )
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The peak live bytes of each built-in metric over the default tree,
/// under a cap high enough to let it finish.
#[test]
#[ignore = "a measurement: run it on demand"]
fn builtin_metric_peaks() {
    let w = ScriptWorkers::new(
        env!("CARGO_BIN_EXE_oxplow-script-worker").into(),
        Limits {
            max_concurrent: 1,
            memory_bytes: 8 * 1024 * 1024 * 1024,
        },
    );
    let tree = Arc::new(fixtures::tree::corpus(fixtures::tree::DEFAULT_FILES));
    for m in oxplow_script::builtin_metrics() {
        let started = Instant::now();
        let ran = w
            .run(
                &Budget::default(),
                ScriptRun::starlark(m.script, json!({})).with_host(RunHost::Tree(tree.clone())),
                None,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", m.key));
        eprintln!(
            "{:40} peak {:>6} MB  cpu {:>6} ms  wall {:>6.0} ms",
            m.key,
            ran.peak_bytes / (1024 * 1024),
            ran.cpu_ms,
            ms(started.elapsed())
        );
    }
}

#[test]
#[ignore = "a measurement: run it on demand"]
fn worker_cost() {
    let w = workers();
    let budget = Budget::default();

    // Cold start: a trivial run, each its own process.
    let mut cold = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        let ran = w.run(&budget, ScriptRun::jaq(".", json!(1)), None).unwrap();
        cold.push((ms(started.elapsed()), ran.cpu_ms));
        assert_eq!(ran.value, json!(1));
    }
    cold.sort_by(|a, b| a.0.total_cmp(&b.0));
    eprintln!(
        "cold trivial run: median {:.1} ms, p90 {:.1} ms (worker cpu {} ms)",
        cold[10].0, cold[18].0, cold[10].1
    );

    // The whole-workspace lcov parse through the bundled report collector.
    let report = fixtures::lcov::report(fixtures::lcov::DEFAULT_LINES);
    let collector = oxplow_script::Collector::bundled("lcov").expect("the bundled lcov parser");
    let started = Instant::now();
    collector.run(&report).expect("parses");
    let lcov_wall = started.elapsed();
    eprintln!(
        "lcov parse ({} KB): {:.0} ms in a worker",
        report.len() / 1024,
        ms(lcov_wall)
    );

    // A sweep of the built-in metrics over one tree, as the fact engine
    // runs them: each run ships the tree, then runs.
    let tree = Arc::new(fixtures::tree::corpus(fixtures::tree::DEFAULT_FILES));
    let tree_bytes: usize = tree.iter().map(|(k, v)| k.len() + v.len()).sum();
    let metrics = oxplow_script::builtin_metrics();
    let mut sweep = Duration::ZERO;
    let mut cpu = 0;
    for m in &metrics {
        let started = Instant::now();
        let ran = w
            .run(
                &budget,
                ScriptRun::starlark(m.script, json!({})).with_host(RunHost::Tree(tree.clone())),
                None,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", m.key));
        sweep += started.elapsed();
        cpu += ran.cpu_ms;
    }
    eprintln!(
        "fact sweep: {} metrics over {} files ({} KB): {:.0} ms wall, {} ms worker cpu, \
         {:.1} ms overhead per run",
        metrics.len(),
        tree.len(),
        tree_bytes / 1024,
        ms(sweep),
        cpu,
        (ms(sweep) - cpu as f64) / metrics.len() as f64
    );
}

/// `oxplow.duplicate_lines` over this repository's own Rust sources: the
/// metric that can grow with duplication, on a real tree.
#[test]
#[ignore = "a measurement: run it on demand"]
fn duplicate_lines_on_this_repo() {
    fn walk(
        dir: &std::path::Path,
        root: &std::path::Path,
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == "target" || name == "node_modules" || name.to_string_lossy().starts_with('.')
            {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else if path
                .extension()
                .is_some_and(|e| e == "rs" || e == "ts" || e == "tsx")
            {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, text);
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = std::collections::HashMap::new();
    walk(&root, &root, &mut files);
    let bytes: usize = files.values().map(String::len).sum();
    let w = ScriptWorkers::new(
        env!("CARGO_BIN_EXE_oxplow-script-worker").into(),
        Limits {
            max_concurrent: 1,
            memory_bytes: 8 * 1024 * 1024 * 1024,
        },
    );
    let metric = oxplow_script::builtin_metrics()
        .into_iter()
        .find(|m| m.key == "oxplow.duplicate_lines")
        .unwrap();
    let started = Instant::now();
    let ran = w
        .run(
            &Budget::default(),
            ScriptRun::starlark(metric.script, json!({}))
                .with_host(RunHost::Tree(Arc::new(files.clone()))),
            None,
        )
        .unwrap();
    eprintln!(
        "duplicate_lines over {} files ({} MB): peak {} MB, cpu {} ms, wall {:.0} ms, {} facts",
        files.len(),
        bytes / (1024 * 1024),
        ran.peak_bytes / (1024 * 1024),
        ran.cpu_ms,
        ms(started.elapsed()),
        ran.value["facts"].as_array().map_or(0, Vec::len)
    );
}
