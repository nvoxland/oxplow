//! P10 (K3): the documented github example is the first-party use that
//! made `ref_kinds` stable. It checks and tests clean, and its
//! pull-request kind works in a real oxplow: `[[pr:12]]` names
//! `github_pr:12`, which resolves to the pull request's title and opens
//! its `pr` page.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_sdk::{check, plugin_test::test_extension, render_findings, Format};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_github_example_checks_tests_and_its_pr_opens() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/github");
    copy_dir(&example, &root.join("oxplow/extensions/github"));

    let report = check(root, "github", &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "github", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    assert!(
        tested.ran.contains(&"example pr-link".to_string()),
        "{:?}",
        tested.ran
    );

    // A real oxplow over the project, one pull request collected.
    let svc = oxplow_app::Services::in_memory(root).unwrap();
    // Its entity's table, as the collector's first run makes it.
    oxplow_app::collector_runner::publish_declared_empty(&svc.db, &svc.extension_catalog.get(root))
        .await
        .unwrap();
    svc.extension_models.sync().await.unwrap();
    svc.vocabulary_service.sync().await.unwrap();
    svc.db
        .transaction(|tx| {
            tx.execute(
                "INSERT INTO ext__github__pr (number, title, body, state, author, head_branch, \
                 draft, opened_at, merged_at, url) VALUES (12, 'Fix the hover state', \
                 'The button flickered.', 'open', 'octocat', 'fix-hover', 0, \
                 '2026-10-01T00:00:00Z', NULL, 'https://github.com/o/r/pull/12')",
                [],
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();

    let kinds = &svc.vocabulary.current().kinds;
    assert!(kinds.get("github_pr").is_some());
    assert_eq!(
        oxplow_domain::refs::canonical_wikilink(kinds, "pr:12").map(|r| r.to_string()),
        Some("github_pr:12".to_string())
    );
    let row = |sql: &str| {
        let sql = sql.to_string();
        let svc = &svc;
        async move {
            serde_json::to_value(svc.sql.query_sql(&sql, vec![], None).await.unwrap().rows).unwrap()
        }
    };
    assert_eq!(
        row("SELECT page, resolve FROM v_ref_kind WHERE kind = 'github_pr'").await,
        serde_json::json!([["page:ext.github.pr", "v_github_pull_request"]])
    );
    assert_eq!(
        row("SELECT title FROM v_github_pull_request WHERE ref = 'github_pr:12'").await,
        serde_json::json!([["Fix the hover state"]])
    );
    // tsk894: `[[pr:12]]` is a valid link; one to a pull request the
    // model doesn't have is flagged.
    let warnings = oxplow_app::link_check::check_links(&svc, "See [[pr:12]] and [[pr:99]].").await;
    assert_eq!(
        warnings
            .iter()
            .map(|w| w.target.as_str())
            .collect::<Vec<_>>(),
        vec!["pr:99"],
        "{warnings:?}"
    );
    // Its page is the `pr` lens, given `?ref=`.
    let run = oxplow_app::extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        root,
        "github/pr",
        [(
            "ref".to_string(),
            oxplow_db::SqlCell::Text("github_pr:12".into()),
        )]
        .into_iter()
        .collect(),
        &oxplow_app::extensions::LensContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(run.result.rows.len(), 1);
    let shown = serde_json::to_string(&run.result.rows).unwrap();
    assert!(shown.contains("Fix the hover state"), "{shown}");
}
