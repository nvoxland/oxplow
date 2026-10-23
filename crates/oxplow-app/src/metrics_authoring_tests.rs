//! An agent-authored code metric, end to end: the scaffolded extension
//! (`oxplow extension new metric`) over a small Rust corpus in two
//! folders. It activates cleanly; one capture records a fact per file with
//! its area; edits to two files and a second capture (a delta) carry the
//! untouched files' facts forward; the metric slices by area, filters by a
//! dimension and reads through the lens as `file` refs; and what an author
//! gets wrong, or what a collector is never shown, is reported rather than
//! lost.

use std::path::Path;
use std::sync::Arc;

use oxplow_code_metrics::Language;
use oxplow_db::SqlCell;
use oxplow_domain::StreamId;

use crate::metric_scaffold::{extension_files, source, MetricStarter, ScaffoldFile};
use crate::metrics_service::{BaselineReport, SweepReport};

const NAME: &str = "complex-fns";
const MEASURE: &str = "complex_fns.complex_functions";
const AREA: &str = "complex_fns.area";
const COLLECTOR: &str = "complex_fns.scan";

/// A Rust file defining each `(name, branches)` function; the line each
/// starts on. `branches` of 11 is over the starter's limit of 10.
fn rust(functions: &[(&str, usize)]) -> (String, Vec<usize>) {
    source(Language::Rust, functions)
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// The corpus: two folders, four files.
///
/// | file            | functions over the limit | worst starts at |
/// |-----------------|--------------------------|-----------------|
/// | engine/run.rs   | 1                        | `busy`          |
/// | engine/plan.rs  | 2                        | `busy_b`        |
/// | ui/view.rs      | 1                        | `busy`          |
/// | ui/theme.rs     | 0                        | (no fact)       |
fn corpus(root: &Path) -> Lines {
    let (run, run_at) = rust(&[("calm", 0), ("busy", 11)]);
    let (plan, plan_at) = rust(&[("busy_a", 11), ("busy_b", 12)]);
    let (view, view_at) = rust(&[("calm", 0), ("busy", 11)]);
    let (theme, _) = rust(&[("calm", 0)]);
    write(root, "engine/run.rs", &run);
    write(root, "engine/plan.rs", &plan);
    write(root, "ui/view.rs", &view);
    write(root, "ui/theme.rs", &theme);
    Lines {
        run: run_at[1],
        plan: plan_at[1],
        view: view_at[1],
    }
}

/// The line the worst function of each file starts on.
struct Lines {
    run: usize,
    plan: usize,
    view: usize,
}

struct Project {
    dir: tempfile::TempDir,
    svc: Arc<crate::Services>,
}

/// A git project holding the corpus and the scaffolded extension (after
/// `edit`ing its files, as an author would), activated as at boot.
async fn project(edit: impl FnOnce(&mut Vec<ScaffoldFile>)) -> (Project, Lines) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let lines = corpus(dir.path());
    let mut files = extension_files(
        NAME,
        Some("oxplow:oxplow/effort?id=eff1"),
        &MetricStarter::default(),
    );
    edit(&mut files);
    for f in &files {
        write(dir.path(), &f.path, &f.content);
    }
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.streams.ensure_primary().await.unwrap();
    crate::test_fixtures::activate_with_problems(&svc).await;
    (Project { dir, svc }, lines)
}

/// The scaffold's file `suffix`, for an edit.
fn file<'a>(files: &'a mut [ScaffoldFile], suffix: &str) -> &'a mut ScaffoldFile {
    files
        .iter_mut()
        .find(|f| f.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no {suffix}"))
}

impl Project {
    /// The first capture: a snapshot of the tree and the whole-tree baseline
    /// of every collector (forced, since a fresh project has no snapshot yet
    /// to find a collector wanting one).
    async fn baseline(&self) -> BaselineReport {
        self.svc.metrics.rebuild_baseline(true).await.unwrap()
    }

    /// A later capture: what the edits since changed, snapshotted, and the
    /// collectors' delta over exactly those files.
    async fn delta(&self) -> SweepReport {
        let capture = self.svc.snapshot_captures.primary().expect("a capture");
        capture.await_initial_ready().await;
        capture.enqueue_startup_diff().await.unwrap();
        let snapshot = capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Manual)
            .await
            .unwrap()
            .expect("a snapshot of the edits");
        self.svc
            .metrics
            .run_snapshot_collectors(StreamId::new(1), snapshot, false, None, &|_| true, None)
            .await
    }

    async fn rows(&self, sql: &str) -> Vec<Vec<SqlCell>> {
        self.svc
            .sql
            .query_sql(sql, Vec::new(), None)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .rows
    }

    /// `(path, line, count)` of every file the metric currently finds.
    async fn findings(&self) -> Vec<(String, i64, i64)> {
        self.rows(&format!(
            "SELECT path, line, CAST(value AS INTEGER) FROM metric_findings('{MEASURE}') ORDER BY path"
        ))
        .await
        .into_iter()
        .map(|r| (text(&r[0]), int(&r[1]), int(&r[2])))
        .collect()
    }

    async fn headline(&self) -> f64 {
        let spec = self
            .svc
            .fact_store
            .get_spec(MEASURE)
            .await
            .unwrap()
            .expect("the metric is in the catalog");
        self.svc
            .metric_engine
            .headline_for_spec(&spec)
            .await
            .unwrap()
            .expect("a headline")
    }

    /// The collector's last run, as `v_collector_run` says.
    async fn run_status(&self) -> (String, Option<String>) {
        let rows = self
            .rows(&format!(
                "SELECT status, error FROM v_collector_run WHERE owner = '{NAME}' AND id = '{COLLECTOR}'"
            ))
            .await;
        let row = rows.first().expect("the collector ran");
        (text(&row[0]), opt_text(&row[1]))
    }
}

fn text(c: &SqlCell) -> String {
    match c {
        SqlCell::Text(t) => t.clone(),
        other => panic!("not text: {other:?}"),
    }
}

fn opt_text(c: &SqlCell) -> Option<String> {
    match c {
        SqlCell::Null(()) => None,
        other => Some(text(other)),
    }
}

fn int(c: &SqlCell) -> i64 {
    match c {
        SqlCell::Int(i) => *i,
        SqlCell::Real(r) => *r as i64,
        other => panic!("not a number: {other:?}"),
    }
}

fn fs(path: &str, line: usize, n: i64) -> (String, i64, i64) {
    (path.to_string(), line as i64, n)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scaffolded_metric_activates_and_the_first_capture_records_a_fact_per_file_with_its_area(
) {
    let (p, at) = project(|_| {}).await;
    let problems = p.svc.extension_activation.problems();
    assert!(problems.is_empty(), "{problems:?}");

    let report = p.baseline().await;
    assert!(report.ran, "{report:?}");
    assert_eq!(report.failed, Vec::<String>::new());

    // One fact per file that has functions over the limit, none for the
    // clean one, each at its worst function's line.
    assert_eq!(
        p.findings().await,
        vec![
            fs("engine/plan.rs", at.plan, 2),
            fs("engine/run.rs", at.run, 1),
            fs("ui/view.rs", at.view, 1),
        ]
    );
    assert_eq!(p.headline().await, 4.0);

    // Each fact carries its area: the top folder of its path.
    let dims = p
        .rows(&format!(
            "SELECT path, json_extract(dims_json, '$.\"{AREA}\"') FROM v_fact \
             WHERE measure_key = '{MEASURE}' ORDER BY path"
        ))
        .await;
    let dims: Vec<(String, String)> = dims.iter().map(|r| (text(&r[0]), text(&r[1]))).collect();
    assert_eq!(
        dims,
        [
            ("engine/plan.rs".to_string(), "engine".to_string()),
            ("engine/run.rs".to_string(), "engine".to_string()),
            ("ui/view.rs".to_string(), "ui".to_string()),
        ]
    );
    assert_eq!(p.run_status().await, ("ok".to_string(), None));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delta_capture_carries_forward_the_files_it_did_not_touch() {
    let (p, at) = project(|_| {}).await;
    p.baseline().await;
    assert_eq!(p.headline().await, 4.0);

    // Edit two files: plan.rs is cleaned up, theme.rs gains three.
    let (plan, _) = rust(&[("calm", 0)]);
    let (theme, theme_at) = rust(&[("calm", 0), ("a", 11), ("b", 11), ("c", 13)]);
    write(p.dir.path(), "engine/plan.rs", &plan);
    write(p.dir.path(), "ui/theme.rs", &theme);
    let sweep = p.delta().await;
    assert_eq!(sweep.failed, Vec::<String>::new());
    // The first capture scanned the whole tree; this one only the edits.
    let kinds = p
        .rows(&format!(
            "SELECT scan_kind FROM v_capture WHERE producer = '{COLLECTOR}' ORDER BY id"
        ))
        .await;
    let kinds: Vec<String> = kinds.iter().map(|r| text(&r[0])).collect();
    assert_eq!(kinds, ["full", "delta"], "{sweep:?}");

    // run.rs and view.rs were not rescanned and keep their facts; plan.rs
    // was rescanned clean, so its old fact is superseded; theme.rs is new.
    assert_eq!(
        p.findings().await,
        vec![
            fs("engine/run.rs", at.run, 1),
            fs("ui/theme.rs", theme_at[3], 3),
            fs("ui/view.rs", at.view, 1),
        ]
    );
    assert_eq!(p.headline().await, 5.0);

    // The series across the two captures: 4, then 5.
    let series: Vec<i64> = p
        .rows(&format!(
            "SELECT CAST(\"measure:{MEASURE}\" AS INTEGER) FROM (\
               SELECT capture_id, MEASURE('{MEASURE}') AS \"measure:{MEASURE}\" \
               FROM metric_grid('capture')) ORDER BY capture_id"
        ))
        .await
        .iter()
        .map(|r| int(&r[0]))
        .collect();
    assert_eq!(series, [4, 5]);
}

/// `v_metric_point` is the metric grid's capture rows for every metric at
/// once: the same series, one metric when pinned, sliced when it pins a
/// dimension.
#[tokio::test(flavor = "multi_thread")]
async fn the_metric_points_are_the_series_for_every_metric_pinned_or_sliced() {
    let (p, _) = project(|_| {}).await;
    p.baseline().await;
    let (plan, _) = rust(&[("calm", 0)]);
    let (theme, _) = rust(&[("calm", 0), ("a", 11), ("b", 11), ("c", 13)]);
    write(p.dir.path(), "engine/plan.rs", &plan);
    write(p.dir.path(), "ui/theme.rs", &theme);
    p.delta().await;

    let pinned: Vec<i64> = p
        .rows(&format!(
            "SELECT CAST(value AS INTEGER) FROM v_metric_point WHERE metric = '{MEASURE}' ORDER BY captured_at"
        ))
        .await
        .iter()
        .map(|r| int(&r[0]))
        .collect();
    assert_eq!(pinned, [4, 5]);
    // Unpinned, every metric with a spec is listed, this one among them.
    let metrics = p.rows("SELECT DISTINCT metric FROM v_metric_point").await;
    assert!(
        metrics.iter().any(|r| text(&r[0]) == MEASURE),
        "{metrics:?}"
    );
    // Sliced by area: one row per capture and area.
    let sliced = p
        .rows(&format!(
            "SELECT capture_id, group_value, CAST(value AS INTEGER) FROM v_metric_point \
             WHERE metric = '{MEASURE}' AND dimension = '{AREA}' ORDER BY capture_id, group_value"
        ))
        .await;
    let latest: Vec<(String, i64)> = sliced
        .iter()
        .filter(|r| int(&r[0]) == int(&sliced.last().unwrap()[0]))
        .map(|r| (text(&r[1]), int(&r[2])))
        .collect();
    assert_eq!(latest, [("engine".to_string(), 1), ("ui".to_string(), 4)]);
}

/// The Metrics page: each metric a row with its trend (the recent values,
/// oldest first, as JSON), its latest value and its section; expanding a
/// row gives its biggest groups under the dimension picked.
#[tokio::test(flavor = "multi_thread")]
async fn the_metrics_list_has_each_metrics_trend_and_its_groups_when_sliced() {
    let (p, _) = project(|_| {}).await;
    p.baseline().await;
    let (plan, _) = rust(&[("calm", 0)]);
    let (theme, _) = rust(&[("calm", 0), ("a", 11), ("b", 11), ("c", 13)]);
    write(p.dir.path(), "engine/plan.rs", &plan);
    write(p.dir.path(), "ui/theme.rs", &theme);
    p.delta().await;

    let run = |lens: &str, params: Vec<(&str, SqlCell)>| {
        let svc = p.svc.clone();
        let root = p.dir.path().to_path_buf();
        let lens = format!("oxplow-foundation/{lens}");
        let params: std::collections::BTreeMap<String, SqlCell> = params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        async move {
            let run = crate::extensions::run_lens(
                &svc.sql,
                &svc.extension_catalog,
                &root,
                &lens,
                params,
                &crate::extensions::LensContext::default(),
            )
            .await
            .unwrap();
            let cols = run.result.columns.clone();
            run.result
                .rows
                .into_iter()
                .map(|r| {
                    cols.iter()
                        .cloned()
                        .zip(r)
                        .collect::<std::collections::BTreeMap<String, SqlCell>>()
                })
                .collect::<Vec<_>>()
        }
    };
    let rows = run("metrics", vec![]).await;
    let mine = rows
        .iter()
        .find(|r| r["key"] == SqlCell::Text(MEASURE.into()))
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(text_of(&mine["trend"]), "[4.0,5.0]");
    assert_eq!(mine["latest"], SqlCell::Real(5.0));
    assert!(!text_of(&mine["section"]).is_empty());
    // Expanded under a slice: the latest value per area, biggest first.
    let groups = run(
        "metrics-slices",
        vec![
            ("key", SqlCell::Text(MEASURE.into())),
            ("slice", SqlCell::Text(AREA.into())),
        ],
    )
    .await;
    let groups: Vec<(String, SqlCell)> = groups
        .iter()
        .map(|r| (text_of(&r["title"]), r["latest"].clone()))
        .collect();
    assert_eq!(
        groups,
        [
            ("ui".to_string(), SqlCell::Real(4.0)),
            ("engine".to_string(), SqlCell::Real(1.0))
        ]
    );
}

fn text_of(cell: &SqlCell) -> String {
    match cell {
        SqlCell::Text(t) => t.clone(),
        other => panic!("not text: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_metric_slices_by_area_and_filters_by_a_dimension() {
    let (p, _) = project(|_| {}).await;
    p.baseline().await;
    let (plan, _) = rust(&[("calm", 0)]);
    let (theme, _) = rust(&[("calm", 0), ("a", 11), ("b", 11), ("c", 13)]);
    write(p.dir.path(), "engine/plan.rs", &plan);
    write(p.dir.path(), "ui/theme.rs", &theme);
    p.delta().await;

    // The latest day's value per area.
    let by_area = p
        .rows(&format!(
            "SELECT \"{AREA}\", CAST(MEASURE('{MEASURE}') AS INTEGER) \
             FROM metric_grid('day', '{AREA}') ORDER BY \"{AREA}\""
        ))
        .await;
    let by_area: Vec<(String, i64)> = by_area.iter().map(|r| (text(&r[0]), int(&r[1]))).collect();
    // engine: run.rs (1); ui: view.rs (1) + theme.rs (3).
    assert_eq!(by_area, [("engine".to_string(), 1), ("ui".to_string(), 4)]);

    // A dimension filter keeps one area's slice.
    let ui = p
        .rows(&format!(
            "SELECT CAST(MEASURE('{MEASURE}') AS INTEGER) \
             FROM metric_grid('day', '{AREA}') WHERE \"{AREA}\" = 'ui'"
        ))
        .await;
    assert_eq!(ui.iter().map(|r| int(&r[0])).collect::<Vec<_>>(), [4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lens_rows_carry_file_refs_to_the_path_and_line() {
    let (p, at) = project(|_| {}).await;
    p.baseline().await;

    let ext = p
        .svc
        .extension_catalog
        .unpinned(p.dir.path(), "a test reads declarations")
        .iter()
        .find(|e| e.name == NAME)
        .cloned()
        .expect("the extension loads");
    assert_eq!(ext.errors, Vec::<String>::new());
    let lens = &ext.lenses[0];
    let run = crate::extensions::run_lens(
        &p.svc.sql,
        &p.svc.extension_catalog,
        p.dir.path(),
        &lens.id,
        Default::default(),
        &crate::extensions::LensContext::default(),
    )
    .await
    .unwrap();

    // `path` links as a `file` at the `line` column.
    let link = run
        .lens
        .columns
        .iter()
        .find_map(|c| c.link.as_ref().map(|l| (c.key.as_str(), l)))
        .expect("a linked column");
    assert_eq!(link.0, "path");
    assert_eq!(link.1.kind, crate::extensions::LensLinkKind::File);
    let (path_col, line_col) = (
        run.result.columns.iter().position(|c| c == "path").unwrap(),
        run.result
            .columns
            .iter()
            .position(|c| Some(c.as_str()) == link.1.line.as_deref())
            .unwrap(),
    );
    // Each row names a file and a line that resolve as a canonical ref.
    let refs: Vec<String> = run
        .result
        .rows
        .iter()
        .map(|r| {
            let r#ref = oxplow_domain::uri::build::file_uri_at(
                &text(&r[path_col]),
                None,
                Some(&int(&r[line_col]).to_string()),
            );
            oxplow_domain::uri::OxplowUri::parse(&r#ref)
                .unwrap_or_else(|e| panic!("{}: {}", r#ref, e.reason()));
            r#ref
        })
        .collect();
    // Worst first, then by path.
    assert_eq!(
        refs,
        [
            oxplow_domain::uri::build::file_uri_at(
                "engine/plan.rs",
                None,
                Some(&at.plan.to_string())
            ),
            oxplow_domain::uri::build::file_uri_at(
                "engine/run.rs",
                None,
                Some(&at.run.to_string())
            ),
            oxplow_domain::uri::build::file_uri_at("ui/view.rs", None, Some(&at.view.to_string())),
        ]
    );
}

/// What the author gets wrong fails the run with the documented reason, so
/// the metric reads stale and says why instead of recording nothing quietly.
#[tokio::test(flavor = "multi_thread")]
async fn a_collector_that_breaks_its_declarations_fails_with_the_documented_error() {
    // (the edit to the script, what the error says)
    let cases: [(&str, &str, &str); 3] = [
        // A bare dimension key: namespaced keys only.
        (
            "\"dims\": {\"complex_fns.area\": area(f[\"path\"])}",
            "\"dims\": {\"area\": area(f[\"path\"])}",
            "dimension key `area` isn't namespaced",
        ),
        // A measure the collector doesn't list in `facts:`.
        (
            "\"measure\": \"complex_fns.complex_functions\"",
            "\"measure\": \"complex_fns.other\"",
            "a fact for `complex_fns.other`, which isn't in the collector's `facts:`",
        ),
        // A measure no one defines.
        (
            "\"measure\": \"complex_fns.complex_functions\"",
            "\"measure\": \"complex_fns.ghost\"",
            "a fact for `complex_fns.ghost`, which no measure defines (declare it in `measures:`)",
        ),
    ];
    for (i, (from, to, says)) in cases.into_iter().enumerate() {
        let (p, _) = project(|files| {
            let script = file(files, "complex_functions.star");
            assert!(script.content.contains(from), "{from}");
            script.content = script.content.replace(from, to);
            // The last case declares the ghost in `facts:` but not in `measures:`.
            if i == 2 {
                let manifest = file(files, "extension.yaml");
                manifest.content = manifest.content.replace(
                    "facts: [complex_fns.complex_functions]",
                    "facts: [complex_fns.complex_functions, complex_fns.ghost]",
                );
            }
        })
        .await;
        let report = p.baseline().await;
        assert_eq!(report.failed, [COLLECTOR], "{says}: {report:?}");
        let (status, error) = p.run_status().await;
        assert_eq!(status, "error", "{says}");
        let error = error.expect("the run says why");
        assert!(error.contains(says), "want `{says}` in `{error}`");
        // Nothing half-recorded: the metric has no facts.
        assert_eq!(
            p.rows(&format!(
                "SELECT count(*) FROM v_fact WHERE measure_key = '{MEASURE}'"
            ))
            .await[0][0],
            SqlCell::Int(0),
            "{says}"
        );
    }
}

/// A namespaced dimension key needs no `dimensions:` entry: only a bare key
/// is refused. The undeclared one is recorded and slices like a declared
/// one; the entry just names it in the catalog (`v_dimension`).
#[tokio::test(flavor = "multi_thread")]
async fn a_namespaced_dimension_key_need_not_be_declared_to_slice() {
    let (p, _) = project(|files| {
        let script = file(files, "complex_functions.star");
        script.content = script
            .content
            .replace("complex_fns.area", "complex_fns.team");
    })
    .await;
    let report = p.baseline().await;
    assert_eq!(report.failed, Vec::<String>::new());
    let by_team = p
        .rows(
            "SELECT \"complex_fns.team\", CAST(MEASURE('complex_fns.complex_functions') AS INTEGER) \
             FROM metric_grid('day', 'complex_fns.team') ORDER BY 1",
        )
        .await;
    let by_team: Vec<(String, i64)> = by_team.iter().map(|r| (text(&r[0]), int(&r[1]))).collect();
    assert_eq!(by_team, [("engine".to_string(), 3), ("ui".to_string(), 1)]);
    let declared = p
        .rows("SELECT key FROM v_dimension WHERE key LIKE 'complex_fns.%' ORDER BY key")
        .await;
    assert_eq!(
        declared.iter().map(|r| text(&r[0])).collect::<Vec<_>>(),
        ["complex_fns.area"],
        "only the declared one is in the catalog"
    );
}

/// A file over `snapshotMaxFileBytes` is not given to collectors; the run
/// says which files it left out and why, rather than reading as a clean file.
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_file_is_reported_as_excluded_not_silently_dropped() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let at = corpus(dir.path());
    // A file bigger than the limit, with a function that would count.
    let mut huge = rust(&[("busy", 11)]).0;
    huge.push_str(&"// padding\n".repeat(1100));
    write(dir.path(), "engine/huge.rs", &huge);
    // A binary file is not text a collector can read either.
    std::fs::write(dir.path().join("ui/icon.rs"), [0u8, 159, 146, 150]).unwrap();
    write(
        dir.path(),
        ".oxplow/project.yaml",
        "snapshotMaxFileBytes: 8000\n",
    );
    assert!(huge.len() > 8000);
    for f in extension_files(NAME, None, &MetricStarter::default()) {
        write(dir.path(), &f.path, &f.content);
    }
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.streams.ensure_primary().await.unwrap();
    crate::test_fixtures::activate_with_problems(&svc).await;
    let p = Project { dir, svc };

    let report = p.baseline().await;
    assert_eq!(report.failed, Vec::<String>::new());
    // The huge file has no fact...
    assert_eq!(
        p.findings().await,
        vec![
            fs("engine/plan.rs", at.plan, 2),
            fs("engine/run.rs", at.run, 1),
            fs("ui/view.rs", at.view, 1),
        ]
    );
    // ...and the report names it, with why.
    let excluded: Vec<(&str, &str)> = report
        .excluded
        .iter()
        .map(|e| (e.path.as_str(), e.reason.as_str()))
        .collect();
    assert_eq!(
        excluded,
        [("engine/huge.rs", "oversize"), ("ui/icon.rs", "binary")]
    );
}

/// A project whose snapshots keep files over `max` bytes by stat only.
async fn limited_project(max: usize) -> (Project, Lines) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let lines = corpus(dir.path());
    write(
        dir.path(),
        ".oxplow/project.yaml",
        &format!("snapshotMaxFileBytes: {max}\n"),
    );
    for f in extension_files(NAME, None, &MetricStarter::default()) {
        write(dir.path(), &f.path, &f.content);
    }
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.streams.ensure_primary().await.unwrap();
    crate::test_fixtures::activate_with_problems(&svc).await;
    (Project { dir, svc }, lines)
}

impl Project {
    /// `unchecked` of every exclusion of `path`.
    async fn unchecked(&self, path: &str) -> Vec<Vec<SqlCell>> {
        self.rows(&format!(
            "SELECT unchecked FROM v_capture_exclusion WHERE path = '{path}'"
        ))
        .await
    }

    /// `(path, reason)` the latest capture left out, as `v_capture_exclusion` says.
    async fn left_out(&self) -> Vec<(String, String)> {
        self.rows(
            "SELECT path, reason FROM v_capture_exclusion
              WHERE capture_id = (SELECT max(id) FROM v_capture) ORDER BY path",
        )
        .await
        .iter()
        .map(|r| (text(&r[0]), text(&r[1])))
        .collect()
    }
}

/// A file that becomes excluded between two sweeps (it grew past the size
/// limit) is not read as clean: its last good facts stay, the capture names
/// it, and once it is readable again the next sweep restates it.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_grows_past_the_limit_keeps_its_facts_and_is_named() {
    let (p, at) = limited_project(8000).await;
    p.baseline().await;
    let before = vec![
        fs("engine/plan.rs", at.plan, 2),
        fs("engine/run.rs", at.run, 1),
        fs("ui/view.rs", at.view, 1),
    ];
    assert_eq!(p.findings().await, before);
    assert_eq!(p.left_out().await, Vec::<(String, String)>::new());

    // engine/run.rs grows past the limit: the sweep is not given it.
    let (run, _) = rust(&[("calm", 0), ("busy", 11)]);
    write(
        p.dir.path(),
        "engine/run.rs",
        &format!("{run}{}", "// padding\n".repeat(1100)),
    );
    let swept = p.delta().await;
    assert_eq!(swept.failed, Vec::<String>::new());
    assert_eq!(
        swept
            .excluded
            .iter()
            .map(|e| (e.path.as_str(), e.reason.as_str()))
            .collect::<Vec<_>>(),
        [("engine/run.rs", "oversize")]
    );
    // Its facts are the last good ones, not a clean zero…
    assert_eq!(p.findings().await, before);
    // …and the capture says it was not checked.
    assert_eq!(
        p.left_out().await,
        [("engine/run.rs".to_string(), "oversize".to_string())]
    );

    // Every collector of the sweep left it out; none has scanned it since.
    let flags = p.unchecked("engine/run.rs").await;
    assert!(!flags.is_empty() && flags.iter().all(|r| int(&r[0]) == 1));

    // Readable again (and now worse): the next sweep restates it.
    let (worse, worse_at) = rust(&[("calm", 0), ("busy", 11), ("busier", 12)]);
    write(p.dir.path(), "engine/run.rs", &worse);
    p.delta().await;
    assert_eq!(p.left_out().await, Vec::<(String, String)>::new());
    // The old exclusion is history now: a later scan covered the file.
    let flags = p.unchecked("engine/run.rs").await;
    assert!(!flags.is_empty() && flags.iter().all(|r| int(&r[0]) == 0));
    assert_eq!(
        p.findings().await,
        [
            fs("engine/plan.rs", at.plan, 2),
            fs("engine/run.rs", worse_at[2], 2),
            fs("ui/view.rs", at.view, 1),
        ]
    );
}

/// A metric's facts join other entities in a lens: here, the files an
/// effort changed beside how many functions over the limit each has.
#[tokio::test(flavor = "multi_thread")]
async fn a_lens_joins_metric_facts_with_an_efforts_files() {
    let (p, _) = project(|files| {
        let manifest = files
            .iter()
            .find(|f| f.path.ends_with("extension.yaml"))
            .map(|f| f.path.clone())
            .unwrap();
        let dir = manifest.trim_end_matches("extension.yaml").to_string();
        files.push(ScaffoldFile {
            path: format!("{dir}lenses/touched.yaml"),
            content: format!(
                "title: Touched and complex\ndescription: \"The files an effort changed, with their functions over the limit.\"\nquery: |\n  SELECT f.path AS path, f.value AS functions, e.change_kind AS change\n  FROM metric_findings('{MEASURE}') f\n  JOIN v_effort_file e ON e.path = f.path\n  ORDER BY f.path\nviz: table\ncolumns:\n  - {{ key: path, label: File }}\n  - {{ key: functions, label: Functions over the limit }}\n  - {{ key: change, label: Change }}\n"
            ),
        });
    })
    .await;
    p.baseline().await;
    let opened = p
        .svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            crate::commands::effort::OPEN,
            serde_json::json!({ "thread": oxplow_domain::ThreadId::new(1).to_string() }),
            false,
        )
        .await
        .unwrap();
    let effort = opened.result["effort"]
        .as_str()
        .unwrap()
        .parse::<oxplow_domain::EffortId>()
        .expect("an effort");
    let effort = effort.value();
    p.svc
        .db
        .transaction(move |tx| {
            tx.execute_batch(&format!(
                "INSERT INTO effort_file (effort_id, path, change_kind, local_snapshot_id, source)
                   VALUES ({effort}, 'engine/plan.rs', 'updated', 0, 'claimed'),
                          ({effort}, 'ui/theme.rs', 'created', 0, 'claimed');"
            ))
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();

    let ext = p
        .svc
        .extension_catalog
        .unpinned(p.dir.path(), "a test reads declarations")
        .iter()
        .find(|e| e.name == NAME)
        .cloned()
        .expect("the extension loads");
    assert_eq!(ext.errors, Vec::<String>::new());
    let lens = ext
        .lenses
        .iter()
        .find(|l| l.id.ends_with("touched"))
        .expect("the joining lens");
    let run = crate::extensions::run_lens(
        &p.svc.sql,
        &p.svc.extension_catalog,
        p.dir.path(),
        &lens.id,
        Default::default(),
        &crate::extensions::LensContext::default(),
    )
    .await
    .unwrap();
    // theme.rs changed but has no function over the limit: no fact, no row.
    let rows: Vec<(String, i64, String)> = run
        .result
        .rows
        .iter()
        .map(|r| (text(&r[0]), int(&r[1]), text(&r[2])))
        .collect();
    assert_eq!(
        rows,
        [("engine/plan.rs".to_string(), 2, "updated".to_string())]
    );
}
