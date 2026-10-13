//! P10 (K3): the documented github example is the first-party use that
//! made `ref_kinds` stable. It checks and tests clean, and its
//! pull-request kind works in a real oxplow: `[[pr:12]]` names
//! `github_pr:12`, which resolves to the pull request's title and opens
//! its `pr` page.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_sdk::{check, extension_test::test_extension, render_findings, Format};

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
    // Its entity is created empty at activation, as before a first run.
    let svc = oxplow_app::Services::in_memory(root).await.unwrap();
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

    // P11 (tsk962): PR Lifetimes — the component made `custom_components`
    // stable. Its lens filters by state; the component acts (it runs the
    // sync), so it's a program on Programs, unapproved until a person
    // approves it.
    svc.db
        .transaction(|tx| {
            tx.execute(
                "INSERT INTO ext__github__pr (number, title, body, state, author, head_branch, \
                 draft, opened_at, merged_at, url) VALUES (11, 'Speed up search', '', 'closed', \
                 'octocat', 'fast-search', 0, '2026-09-20T00:00:00Z', '2026-09-24T00:00:00Z', \
                 'https://github.com/o/r/pull/11')",
                [],
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    let lifetimes = |state: &str| {
        let params = [("state".to_string(), oxplow_db::SqlCell::Text(state.into()))]
            .into_iter()
            .collect();
        let svc = &svc;
        async move {
            let run = oxplow_app::extensions::run_lens(
                &svc.sql,
                &svc.extension_catalog,
                root,
                "github/pr-lifetimes",
                params,
                &oxplow_app::extensions::LensContext::default(),
            )
            .await
            .unwrap();
            assert_eq!(
                run.lens.custom.as_ref().unwrap().component.as_deref(),
                Some("pr-lifetimes")
            );
            run.result
                .rows
                .iter()
                .map(|r| serde_json::to_value(&r[0]).unwrap())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        lifetimes("all").await,
        [serde_json::json!(12), serde_json::json!(11)]
    );
    assert_eq!(lifetimes("open").await, [serde_json::json!(12)]);
    assert_eq!(lifetimes("merged").await, [serde_json::json!(11)]);
    let config = svc.config.read().unwrap().clone();
    let programs = oxplow_app::exec_consent::list(
        &svc.approvals,
        root,
        &config,
        svc.extension_catalog.get(root).as_ref(),
    );
    let component = programs
        .iter()
        .find(|p| p.kind == oxplow_app::exec_consent::ProgramKind::Component)
        .expect("the component is on Programs");
    assert_eq!(component.name, "github/pr-lifetimes");
    assert_eq!(component.commands, ["oxplow.collector.sync"]);
    assert!(!component.approved);

    // tsk935: `searchable: pull_request` — site search finds the pull
    // request by its title, under its kind, the ref a hit opens.
    svc.assets.sync_search_kinds().await.unwrap();
    svc.assets.all_changed();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let hits = loop {
        let hits = svc
            .search_store
            .search("hover", None, &["github_pr".to_string()], 10)
            .await
            .unwrap();
        if !hits.is_empty() || std::time::Instant::now() > deadline {
            break hits;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(
        hits.iter()
            .map(|h| (h.kind.as_str(), h.ref_id.as_str()))
            .collect::<Vec<_>>(),
        [("github_pr", "12")],
        "{hits:?}"
    );
}

/// The one-effort-per-prompt policy example checks clean, its examples
/// dry-run as their fixtures say, and the effort-policy suite passes over
/// it in a throwaway host.
#[tokio::test(flavor = "multi_thread")]
async fn the_prompt_efforts_example_checks_and_passes_the_policy_suite() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/prompt-efforts");
    copy_dir(&example, &root.join("oxplow/extensions/prompt-efforts"));

    let report = check(
        root,
        "prompt-efforts",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "prompt-efforts", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    for ran in [
        "example a-prompt-that-changed-files",
        "example a-question-only-prompt",
        "example a-prompt-already-covered",
        "example an-item-started",
        "example an-item-finished",
        "effort_policy suite",
    ] {
        assert!(
            tested.ran.contains(&ran.to_string()),
            "{ran}: {:?}",
            tested.ran
        );
    }
}

/// The prompt-efforts example's fixtures are events oxplow could log: its
/// examples run clean against the event envelope, as do its policy's.
#[tokio::test(flavor = "multi_thread")]
async fn the_prompt_efforts_example_tests_clean() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/prompt-efforts");
    copy_dir(&example, &root.join("oxplow/extensions/prompt-efforts"));
    let tested = test_extension(root, "prompt-efforts", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    for ran in [
        "example a-prompt-that-changed-files",
        "example a-question-only-prompt",
        "example a-prompt-already-covered",
        "example an-item-started",
        "example an-item-finished",
    ] {
        assert!(
            tested.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            tested.ran
        );
    }
}

/// The triage example uses all three host services in one fact collector:
/// the tree it implies, the semantic layer and a model it declares. It
/// checks clean and lists them; `test` skips it until a person approves
/// it; and in a real oxplow, approved, one snapshot's TODO becomes a
/// classified fact beside the open work, every service decided on its
/// run.
#[tokio::test(flavor = "multi_thread")]
async fn the_triage_example_reads_the_tree_the_layer_and_a_model() {
    use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/triage");
    copy_dir(&example, &root.join("oxplow/extensions/triage"));

    let report = check(root, "triage", &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    assert_eq!(
        oxplow_sdk::render_services(&report),
        "collector `todos` (collectors/todos.star):\n  \
         sql.read: declared, used\n  \
         tree.read: implied, used\n  \
         ai.compute: declared, used — a person approves it\n\
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
    assert_eq!(tested.errors, Vec::<String>::new());
    assert!(
        !tested.warnings.iter().any(|w| w.contains("calls a model")),
        "{:?}",
        tested.warnings
    );
    // The preview runs it whole: its job against the fixture model, the
    // event its job logs through its effect, its component's reads — in
    // the throwaway's activated generation, listing what it contributes
    // and what only the app shows.
    for example in [
        "summary-published",
        "summary-with-evidence",
        "summary-cancelled",
        "summary-to-the-wiki",
        "decide-a-todo",
        "summary-delivered",
        "todo-board",
        "todos-and-open-work",
        "an-older-snapshot-event",
    ] {
        assert!(
            tested.ran.contains(&format!("example {example}")),
            "{example}: {:?}",
            tested.ran
        );
    }
    assert!(tested.generation.is_some());
    for id in [
        "triage/job/summarize",
        "triage/effect/publish-summary",
        "triage/custom_component/todo-board",
        "triage/command/summary.ask",
    ] {
        assert!(
            tested.contributes.iter().any(|c| c == id),
            "{id}: {:?}",
            tested.contributes
        );
    }

    // It ran in its own throwaway: the project has no database of its own
    // from it.
    assert!(!root.join(".oxplow/local.sqlite").exists());

    // A real oxplow: a source file with a TODO, one open work item, a
    // model that calls everything a bug, and the person's approval.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "// TODO: the cache never expires\nfn a() {}\n",
    )
    .unwrap();
    let svc = oxplow_app::Services::in_memory(root).await.unwrap();
    // The preview's generation contributes what a real one over the same
    // project does.
    let mut real: Vec<String> = svc
        .extension_catalog
        .named(root, "triage")
        .unwrap()
        .contributions
        .iter()
        .map(|d| d.id.to_string())
        .collect();
    let mut previewed = tested.contributes.clone();
    real.sort();
    previewed.sort();
    assert_eq!(previewed, real);
    svc.commands
        .run(
            &oxplow_domain::Actor::Human,
            "oxplow.work_item.create",
            serde_json::json!({ "title": "Expire the cache" }),
            false,
        )
        .await
        .unwrap();
    let reply = serde_json::json!({ "answers": { "label": { "type": "choice", "choice": "bug", "probabilities": { "bug": 0.9, "cleanup": 0.05, "feature": 0.05 } } } });
    let (base, _) = oxplow_ai_fake::mock(
        "/chat/completions",
        200,
        serde_json::json!({ "choices": [{ "message": { "content": reply.to_string() } }], "usage": { "prompt_tokens": 4, "completion_tokens": 2 } }),
    )
    .await;
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "m".into(),
                kind: "openai_compatible".into(),
                base_url: Some(base),
            },
            None,
        )
        .unwrap();
    svc.ai
        .set_role(
            Role::Decide,
            Some(RoleBinding {
                provider: "m".into(),
                model: "x".into(),
            }),
        )
        .unwrap();
    // The provider kind is itself a script a person approves.
    let config = svc.config.read().unwrap().clone();
    let extensions = svc.extension_catalog.get(root);
    let provider = oxplow_app::exec_consent::list(&svc.approvals, root, &config, &extensions)
        .into_iter()
        .find(|p| {
            p.kind == oxplow_app::exec_consent::ProgramKind::AiProvider
                && p.name.ends_with("/openai_compatible")
        })
        .expect("the provider is on Programs");
    oxplow_app::exec_consent::approve_program(
        &svc.approvals,
        root,
        &config,
        &extensions,
        oxplow_app::exec_consent::ProgramKind::AiProvider,
        &provider.name,
        provider.version.as_deref().unwrap(),
    )
    .unwrap();
    let ext = svc.extension_catalog.named(root, "triage").unwrap();
    let hash =
        oxplow_app::collector_runner::approval_hash(&root.join(&ext.path), &ext.collectors[0])
            .unwrap();
    oxplow_app::collector_runner::approve(&svc.approvals, "triage", "todos", &hash).unwrap();
    let stream = svc.streams.list_streams().await.unwrap()[0].id;
    let capture = svc.snapshot_captures.get(&stream).unwrap();
    capture.enqueue_startup_diff().await.unwrap();
    capture
        .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
        .await
        .unwrap();

    let recorded = svc
        .metrics
        .run_collector_by_key(
            "triage",
            "todos",
            Some(stream),
            &oxplow_domain::run::RunContext::core(
                oxplow_domain::Actor::Human,
                oxplow_domain::run::Mode::External,
            ),
        )
        .await
        .unwrap();
    assert_eq!(recorded, 2);
    let rows = svc
        .sql
        .query_sql(
            "SELECT measure_key, value, path, rule FROM v_fact WHERE measure_key LIKE 'triage.%' \
             ORDER BY measure_key",
            vec![],
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&rows.rows).unwrap(),
        serde_json::json!([
            ["triage.open_items", 1.0, null, null],
            ["triage.todos", 1.0, "src/lib.rs", "bug"],
        ])
    );
    let decided = svc
        .sql
        .query_sql(
            "SELECT decisions FROM v_extension_run WHERE contribution = 'triage/collector/todos'",
            vec![],
            None,
        )
        .await
        .unwrap();
    let decided = serde_json::to_value(&decided.rows).unwrap().to_string();
    for scope in ["tree.read", "ai.compute", "sql.read"] {
        assert!(decided.contains(scope), "{scope}: {decided}");
    }
    assert!(!decided.contains("denied"), "{decided}");
}

/// The task-views example replaces the Board, the Tasks list and the editor
/// with no provider of its own. It checks clean and every one of its
/// fixtures passes: the list, the board's reads and its refused update
/// until approved, and the editor's form.
#[tokio::test(flavor = "multi_thread")]
async fn the_task_views_example_checks_and_its_fixtures_pass() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/task-views");
    copy_dir(&example, &root.join("oxplow/extensions/task-views"));

    let report = check(
        root,
        "task-views",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "task-views", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    for name in [
        "compact-list",
        "swimlanes-reads",
        "swimlanes-needs-approval",
        "swimlanes-renames",
        "edit-item",
    ] {
        assert!(
            tested.ran.contains(&format!("example {name}")),
            "{name}: {:?}",
            tested.ran
        );
    }
}
