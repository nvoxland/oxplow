//! The corpus generators are deterministic, the right size and shape, and
//! their output is what the collector paths read.

mod fixtures;

use fixtures::{lcov, tree};

#[test]
fn the_lcov_report_is_deterministic() {
    assert_eq!(lcov::report(5_000), lcov::report(5_000));
    assert_ne!(lcov::report(5_000), lcov::report(5_001));
}

#[test]
fn the_default_lcov_report_is_about_five_megabytes_and_a_hundred_thousand_lines() {
    let report = lcov::report(lcov::DEFAULT_LINES);
    assert_eq!(report.lines().count(), lcov::DEFAULT_LINES);
    let mb = report.len() as f64 / (1024.0 * 1024.0);
    assert!((4.0..7.0).contains(&mb), "{mb:.2} MB");
}

#[test]
fn the_lcov_report_parses_into_whole_records_of_several_files() {
    let report = lcov::report(20_000);
    let parsed = oxplow_script::parse_report("lcov", &report).expect("lcov parses");
    let records = parsed.as_array().expect("an array of records");
    assert!(records.len() >= 5, "{} files", records.len());
    for record in records {
        let n = |key: &str| -> usize {
            record[key][0]
                .as_str()
                .unwrap_or_default()
                .parse()
                .unwrap_or(usize::MAX)
        };
        assert_eq!(record["SF"].as_array().map(Vec::len), Some(1));
        // A key with no entries (a record without functions) is absent.
        let entries = |key: &str| record[key].as_array().map_or(0, Vec::len);
        assert_eq!(entries("DA"), n("LF"));
        assert_eq!(entries("FN"), n("FNF"));
        assert_eq!(entries("FNDA"), n("FNF"));
        assert_eq!(entries("BRDA"), n("BRF"));
        assert!(n("LH") <= n("LF") && n("FNH") <= n("FNF") && n("BRH") <= n("BRF"));
    }
}

#[test]
fn a_small_lcov_report_is_still_whole() {
    for lines in [1, 10, 11, 19, 20, 30] {
        let report = lcov::report(lines);
        assert!(report.lines().count() >= 10, "{lines}");
        assert!(report.ends_with("end_of_record\n"), "{lines}");
    }
}

#[test]
fn the_tree_is_deterministic() {
    assert_eq!(tree::corpus(30), tree::corpus(30));
}

#[test]
fn the_tree_has_files_of_a_hundred_to_three_hundred_lines_in_three_languages() {
    let files = tree::corpus(tree::DEFAULT_FILES);
    assert_eq!(files.len(), tree::DEFAULT_FILES);
    for ext in [".rs", ".ts", ".py"] {
        assert!(files.keys().any(|p| p.ends_with(ext)), "no {ext} file");
    }
    for (path, text) in &files {
        // A file ends on a whole function, so it may run past its target.
        let lines = text.lines().count();
        assert!((100..=330).contains(&lines), "{path}: {lines} lines");
    }
}

#[test]
fn the_tree_has_duplicated_blocks_for_the_detector() {
    let files = tree::corpus(tree::DEFAULT_FILES);
    let blocks = oxplow_code_dup::detect_duplicates(
        files.iter().map(|(p, t)| (p.clone(), t.as_str())),
        oxplow_code_dup::DupOptions {
            min_lines: 10,
            ..Default::default()
        },
    );
    assert!(blocks.len() >= 10, "{} duplicate blocks", blocks.len());
}

/// The native lcov parser (what the bundled `lcov` collector runs) and the
/// jq program it replaced (`oxplow-config`'s `parsers/lcov.jq`, kept as its
/// reference) read the same reports the same way.
mod native_lcov {
    use oxplow_script::runtime::run_jaq;
    use oxplow_script::{parse_report, CollectorKind};

    use super::fixtures::lcov;

    fn by_jaq(report: &str) -> oxplow_coverage::CoverageReport {
        let program = oxplow_config::collectors::bundled_parser("lcov")
            .expect("bundled")
            .program;
        let records = parse_report("lcov", report).expect("records");
        let value = run_jaq(program, &records).expect("jaq");
        match oxplow_script::runtime::value_to_output(CollectorKind::Coverage, value)
            .expect("shape")
        {
            oxplow_script::CollectorOutput::Coverage(c) => c,
            other => panic!("{other:?}"),
        }
    }

    fn same(report: &str) {
        assert_eq!(
            oxplow_script::helpers::lcov_coverage(report),
            by_jaq(report)
        );
    }

    #[test]
    fn a_generated_report_reads_as_the_jq_program_reads_it() {
        same(&lcov::report(30_000));
    }

    #[test]
    fn the_edges_read_as_the_jq_program_reads_them() {
        // A repeated SF (the later record wins), a trailing record with no
        // end_of_record, a junk DA (skipped), a DA with a third field, a
        // missing hits count, summary lines (the first of each key), a
        // record with no SF (dropped), CRLF and indentation.
        same(
            "SF:a.rs\nDA:1,1\nend_of_record\n\
             SF:a.rs\nDA:2,0\nDA:3,5,abc\nDA:junk\nDA:4\nBRF:4\nBRF:9\nBRH:3\nFNF:2\nFNH:1\nend_of_record\n\
             DA:9,9\nend_of_record\r\n\
             SF:b.rs\r\n  DA:7,2  \r\nLF:1\nend_of_record\n\
             SF:c.rs\nDA:5,1",
        );
        same("");
        same("TN:\nend_of_record\n");
    }
}

/// The bundled `lcov` collector reads a 14 MB report well inside a budget
/// the jq program it replaced overran in a debug build (5.5 s there). A
/// completion budget, generous on purpose: a loaded or instrumented run is
/// several times slower than a quiet one.
#[test]
fn the_bundled_lcov_collector_reads_a_big_report_within_a_budget() {
    let report = fixtures::lcov::report(240_000);
    let started = std::time::Instant::now();
    let out = oxplow_script::Collector::bundled("lcov")
        .expect("bundled")
        .run(&report)
        .expect("runs");
    assert!(out.as_coverage().is_some_and(|c| c.files.len() > 100));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}
