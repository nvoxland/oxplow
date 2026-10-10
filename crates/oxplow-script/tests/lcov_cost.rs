//! Where a big lcov's time goes through the bundled `lcov` parser, stage by
//! stage (ignored by default; prints):
//!
//! ```sh
//! cargo nextest run -p oxplow-script --release --run-ignored only --no-capture lcov_cost
//! ```

mod fixtures;

use std::time::Instant;

use oxplow_script::runtime::{run_jaq, value_to_output};
use oxplow_script::Collector;

fn ms(since: Instant) -> u128 {
    since.elapsed().as_millis()
}

#[test]
#[ignore = "a measurement"]
fn lcov_cost() {
    let report = fixtures::lcov::report(240_000);
    let mb = report.len() as f64 / 1048576.0;
    let program = oxplow_config::collectors::bundled_parser("lcov")
        .expect("bundled")
        .program;

    let t = Instant::now();
    let records = oxplow_script::parse_report("lcov", &report).expect("parse");
    let parse = ms(t);

    let t = Instant::now();
    let bytes = serde_json::to_vec(&records).expect("serialize").len();
    let to_json = ms(t);

    // The input marshaling alone: the whole report in, one number out.
    let t = Instant::now();
    run_jaq("length", &records).expect("jaq");
    let marshal = ms(t);

    // The DA entries alone (all the program reads, plus a few totals),
    // projected away by the host before the program runs.
    let t = Instant::now();
    let value = run_jaq(program, &records).expect("jaq");
    let jaq = ms(t);

    let t = Instant::now();
    let out =
        value_to_output(oxplow_script::CollectorKind::Coverage, value.clone()).expect("shape");
    let coerce = ms(t);
    drop(out);

    let t = Instant::now();
    Collector::bundled("lcov")
        .expect("bundled")
        .run(&report)
        .expect("run");
    let whole = ms(t);

    println!(
        "lcov {mb:.1} MB ({} records, {:.1} MB as JSON): parse {parse} ms, to_json {to_json} ms, \
         jaq {jaq} ms (of which marshaling the input {marshal} ms), coerce {coerce} ms; bundled Collector::run (native) {whole} ms",
        records.as_array().map_or(0, Vec::len),
        bytes as f64 / 1048576.0,
    );
}
