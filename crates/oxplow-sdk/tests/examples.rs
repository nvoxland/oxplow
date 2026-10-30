//! What the example extensions declare, seen only through `oxplow_sdk` (a
//! public surface an extension author has): the services the triage example
//! uses, what it contributes, and how review-badges degrades without
//! effort-review. Their behavior is each example's own `fixtures/`, run by
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

/// Both examples copied into one project.
async fn review_project(with_producer: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_sdk::scaffold_example(root, "review-badges", None).unwrap();
    if with_producer {
        oxplow_sdk::scaffold_example(root, "effort-review", None).unwrap();
    }
    dir
}

/// The review-badges example reads effort-review's exported `reviews`
/// model, optionally: with the producer present its pages and slot load;
/// with it disabled, exporting another version or missing, they are
/// withheld, the import says why, and nothing errors.
#[tokio::test(flavor = "multi_thread")]
async fn the_review_badges_example_consumes_effort_reviews_and_degrades_without_them() {
    // Both present.
    let dir = review_project(true).await;
    let root = dir.path();
    let report = check(
        root,
        "review-badges",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(tested.error_lines(), Vec::<String>::new());
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: available"]
    );
    for id in [
        "oxplow:oxplow/page?page=review-badges/review-verdicts",
        "oxplow:oxplow/page?page=review-badges/item-review",
    ] {
        assert!(
            tested.contributes.iter().any(|c| c == id),
            "{id}: {:?}",
            tested.contributes
        );
    }
    for example in ["review-verdicts", "item-review"] {
        assert!(
            tested.ran.contains(&format!("example {example}")),
            "{example}: {:?}",
            tested.ran
        );
    }

    // The producer missing.
    let dir = review_project(false).await;
    let root = dir.path();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    // The pages and the slot are withheld, named in the one warning for
    // the import; the fixtures that test them are skipped, saying why.
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: unavailable (effort-review or its contribution isn't here)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);
    let warning_lines = tested.warning_lines();
    let import_warnings: Vec<_> = warning_lines
        .iter()
        .filter(|w| w.contains("import `reviews`") && !w.contains("fixtures/"))
        .collect();
    assert_eq!(import_warnings.len(), 1, "{:?}", tested.warnings);
    let warning = import_warnings[0];
    assert!(
        warning.contains(
            "optional import `reviews` for `oxplow:oxplow/dataset?dataset=effort-review/reviews` v2 is unavailable: \
             effort-review or its contribution isn't here; withheld: "
        ),
        "{warning}"
    );
    for id in [
        "`oxplow:oxplow/page?page=review-badges/item-review`",
        "`oxplow:oxplow/page?page=review-badges/review-verdicts`",
        "`oxplow:oxplow/slot?slot=review-badges/oxplow-foundation.work-item.sections-0`",
    ] {
        assert!(warning.contains(id), "{id}: {warning}");
    }
    assert!(
        !tested
            .warning_lines()
            .iter()
            .any(|w| w.contains("because an import")),
        "{:?}",
        tested.warnings
    );
    assert_eq!(tested.error_lines(), Vec::<String>::new());
    for (example, page) in [
        ("review-verdicts", "review-verdicts"),
        ("item-review", "item-review"),
    ] {
        assert!(
            tested.skipped_lines().iter().any(|w| w.contains(&format!(
                "fixtures/{example}.yaml: example `{example}` isn't run: \
                 `oxplow:oxplow/page?page=review-badges/{page}` is withheld because import `reviews` is \
                 unavailable (effort-review or its contribution isn't here)"
            ))),
            "{example}: {:?}",
            tested.skipped
        );
    }
    // A person who turned the producer off withholds what reads it in their
    // oxplow; a test still runs the project's extensions, the producer
    // among them.
    let dir = review_project(true).await;
    let root = dir.path();
    std::fs::create_dir_all(root.join(".oxplow")).unwrap();
    std::fs::write(
        root.join(".oxplow/personal.yaml"),
        "disableExtensions: [effort-review]\n",
    )
    .unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: available"]
    );

    // The producer exports another version.
    let dir = review_project(true).await;
    let root = dir.path();
    let manifest = root.join(".oxplow/extensions/effort-review/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let bumped = text
        .replace(
            "    version: 2\n    key: [job_run]",
            "    version: 3\n    key: [job_run]",
        )
        .replace(
            "contribution: oxplow:oxplow/dataset?dataset=effort-review/reviews, version: 2,",
            "contribution: oxplow:oxplow/dataset?dataset=effort-review/reviews, version: 3,",
        );
    assert_ne!(bumped, text);
    std::fs::write(&manifest, bumped).unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: unavailable (effort-review exports v3)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);
}
