//! What the example extensions declare, seen only through `oxplow_sdk` (a
//! public surface an extension author has): the services the triage example
//! uses and what it contributes. Their behavior is each example's own `fixtures/`, run by
//! `every_example_scaffolds_checks_and_tests_clean`.

#![allow(clippy::unwrap_used)]

use oxplow_sdk::{
    check, extension_test::test_extension, render_findings, ExtensionCatalog, Format,
};

/// The triage example declares the host services its scripts use, and
/// `extension test` previews it whole: its job, effect, component and
/// command are what it contributes.
#[tokio::test(flavor = "multi_thread")]
async fn the_triage_example_declares_its_services_and_contributes_its_parts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_sdk::scaffold_example(root, "triage", None).unwrap();

    let report = check(root, "triage", &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    assert_eq!(
        oxplow_sdk::render_services(&report),
        "collector `todos` (collectors/todos.star):\n  \
         sql.read: declared, used\n  \
         tree.read: implied, used\n  \
         ai.compute: declared, used\n\
         command `triage.todos.decide` (commands/decide.star):\n  \
         state.read: declared, used\n\
         command `triage.todos.decisions` (commands/decisions.star):\n  \
         state.read: declared, used\n  \
         sql.read: declared, used\n\
         command `triage.todos.in_file` (commands/todos_in_file.star):\n  \
         sql.read: declared, used\n\
         effect `publish-summary` (effects/publish_summary.star):\n  \
         jobs.read: declared, used\n"
    );
    let tested = test_extension(root, "triage", false).await.unwrap();
    assert_eq!(tested.error_lines(), Vec::<String>::new());
    assert!(
        !tested
            .warning_lines()
            .iter()
            .any(|w| w.contains("calls a model")),
        "{:?}",
        tested.warnings
    );
    assert!(tested.generation.is_some());
    for id in [
        "oxplow:oxplow/job?job=triage/summarize",
        "oxplow:oxplow/effect?effect=triage/publish-summary",
        "oxplow:oxplow/custom_component?custom_component=triage/todo-board",
        "oxplow:oxplow/command?command=triage.summary.ask",
    ] {
        assert!(
            tested.contributes.iter().any(|c| c == id),
            "{id}: {:?}",
            tested.contributes
        );
    }
    // It ran in its own throwaway: the project has no database from it.
    assert!(!root.join(".oxplow/local.sqlite").exists());
}
