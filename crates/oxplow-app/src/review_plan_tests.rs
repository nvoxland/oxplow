//! A review read is one change's: `WHERE change_id = ?` on the review's
//! signal list must reach each source by that change's own rows, never
//! compute every change's first. SQLite doesn't push a filter into a CTE
//! that is read more than once (or `MATERIALIZED`), so a source written that
//! way scans every change on each read: with 1,500 changes of 10 files it
//! took seconds where the pushed-in plan takes milliseconds
//! (`.context/performance.md` "Per-change datasets").

/// The tables keyed by a change (`change_id`): a scan of one in a read of
/// one change is the whole history computed.
const PER_CHANGE: &[&str] = &[
    "change",
    "change_file",
    "change_function",
    "change_import",
    "change_duplicate",
    "change_test_file",
    "change_hunk",
    "file_kind_rule_decl",
];

/// The plan lines (`EXPLAIN QUERY PLAN` details) of `sql`.
async fn plan(svc: &crate::Services, sql: &str) -> Vec<String> {
    let sql = format!("EXPLAIN QUERY PLAN {sql}");
    svc.db
        .read(move |c| {
            let mut st = c.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(3))
                .map_err(oxplow_db::map_sql_err)?;
            rows.collect::<Result<_, _>>()
                .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap()
}

/// Many changes of ten files each, so a plan that scans them is the one
/// that costs.
async fn many_changes(svc: &crate::Services, n: i64) {
    svc.db
        .transaction(move |tx| {
            for id in 1..=n {
                tx.execute(
                    "INSERT INTO change (id, stream_id, kind, target, status, head_revision)
                     VALUES (?1, 1, 'range', ?2, 'done', 'working')",
                    rusqlite::params![id, format!("t{id}")],
                )
                .map_err(oxplow_db::map_sql_err)?;
                for f in 0..10 {
                    tx.execute(
                        "INSERT INTO change_file (change_id, path, status, additions, deletions, is_test)
                         VALUES (?1, ?2, 'modified', 1, 0, ?3)",
                        rusqlite::params![id, format!("src/f{f}.rs"), f % 3 == 0],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
            }
            Ok(())
        })
        .await
        .unwrap();
}

/// The per-change tables a plan scans.
fn scanned(plan: &[String]) -> Vec<String> {
    plan.iter()
        .filter(|line| {
            let line = line.trim_start();
            PER_CHANGE.iter().any(|t| {
                line.strip_prefix("SCAN ")
                    .is_some_and(|rest| rest == *t || rest.starts_with(&format!("{t} ")))
            })
        })
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_one_changes_review_signals_scans_no_change_table() {
    let f = crate::test_fixtures::services_with_thread().await;
    many_changes(&f.svc, 300).await;
    let lines = plan(
        &f.svc,
        "SELECT * FROM v_oxplow_review_signal_shown WHERE change_id = 7",
    )
    .await;
    let bad = scanned(&lines);
    assert!(
        bad.is_empty(),
        "a read of one change scans {bad:?}:\n{}",
        lines.join("\n")
    );
}

/// Each contributor's own plan, so a failure names the source.
#[tokio::test(flavor = "multi_thread")]
async fn reading_one_changes_signals_per_source_scans_no_change_table() {
    let f = crate::test_fixtures::services_with_thread().await;
    many_changes(&f.svc, 300).await;
    let names: Vec<String> = f
        .svc
        .db
        .read(|c| {
            let mut st = c
                .prepare("SELECT name FROM sqlite_master WHERE type = 'view' AND name LIKE 'v_oxplow_review_signal_%' AND name <> 'v_oxplow_review_signal_shown' ORDER BY 1")
                .map_err(oxplow_db::map_sql_err)?;
            let rows = st.query_map([], |r| r.get(0)).map_err(oxplow_db::map_sql_err)?;
            rows.collect::<Result<_, _>>().map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    assert!(names.len() > 5, "{names:?}");
    let mut report = Vec::new();
    for name in names {
        let lines = plan(&f.svc, &format!("SELECT * FROM {name} WHERE change_id = 7")).await;
        let bad = scanned(&lines);
        if !bad.is_empty() {
            report.push(format!("{name}: {bad:?}"));
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_one_changes_facts_scans_no_change_table() {
    let f = crate::test_fixtures::services_with_thread().await;
    many_changes(&f.svc, 300).await;
    let lines = plan(&f.svc, "SELECT * FROM v_change_fact WHERE change_id = 7").await;
    let bad = scanned(&lines);
    assert!(
        bad.is_empty(),
        "a read of one change scans {bad:?}:\n{}",
        lines.join("\n")
    );
}
