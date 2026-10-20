//! P10 (K3): the documented github example is the first-party use that
//! made `ref_kinds` stable. It checks and tests clean, and its
//! pull-request kind works in a real oxplow: `[[pr:12]]` names
//! `github_pr:12`, which resolves to the pull request's title and opens
//! its `pr` page.

#![allow(clippy::unwrap_used)]

mod common;

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
    copy_dir(&example, &root.join(".oxplow/extensions/github"));

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
    let svc = common::booted(root).await;
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
        serde_json::json!([["page:github/pr?ref={ref}", "v_github_pull_request"]])
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
    // sync).
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
            .map(|h| (h.kind.as_str(), h.reference.as_str()))
            .collect::<Vec<_>>(),
        [("github_pr", "github_pr:12")],
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
    copy_dir(&example, &root.join(".oxplow/extensions/prompt-efforts"));

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
    copy_dir(&example, &root.join(".oxplow/extensions/prompt-efforts"));
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
/// checks clean and lists them; `test` runs its fixtures; and in a real
/// oxplow one snapshot's TODO becomes a
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
    copy_dir(&example, &root.join(".oxplow/extensions/triage"));

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
    ] {
        assert!(
            tested.ran.contains(&format!("example {example}")),
            "{example}: {:?}",
            tested.ran
        );
    }
    assert!(tested.generation.is_some());
    for id in [
        "job:triage/summarize",
        "effect:triage/publish-summary",
        "custom_component:triage/todo-board",
        "command:triage.summary.ask",
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
    // model that calls everything a bug.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "// TODO: the cache never expires\nfn a() {}\n",
    )
    .unwrap();
    let svc = common::booted(root).await;
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
            &Role::parse("triage/sort").expect("a profile role"),
            Some(RoleBinding::new("m", "x")),
        )
        .unwrap();
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
            "SELECT decisions FROM v_extension_run WHERE contribution = 'collector:triage/todos'",
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

    // The sort asked for its own profile, which the person bound.
    let calls = svc
        .sql
        .query_sql("SELECT DISTINCT role FROM v_ai_call", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&calls.rows).unwrap(),
        serde_json::json!([["triage/sort"]])
    );

    // Its decisions command reads the TODOs through the metric
    // (`metric_findings('triage.todos')`), as its board's lens does.
    let listed = svc
        .commands
        .run(
            &oxplow_domain::Actor::Human,
            "triage.todos.decisions",
            serde_json::json!({}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        listed.result["result"]["todos"],
        serde_json::json!([{ "path": "src/lib.rs", "line": 1, "kind": "bug", "decision": null }])
    );
}

/// The task-views example replaces the Board, the Tasks list and the editor
/// with no provider of its own. It checks clean and every one of its
/// fixtures passes: the list, the board's reads and its update, and the editor's form.
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
    copy_dir(&example, &root.join(".oxplow/extensions/task-views"));

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
        "swimlanes-refuses-a-missing-item",
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

/// Run `name` as a person on `svc`'s bus.
async fn as_person(
    svc: &oxplow_app::Services,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    svc.commands
        .run(&oxplow_domain::Actor::Human, name, input, false)
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .result
}

/// Poll `sql` until it returns a row, up to ten seconds.
async fn until_row(svc: &oxplow_app::Services, sql: &str) -> Vec<serde_json::Value> {
    for _ in 0..200 {
        let rows = svc.sql.query_sql(sql, vec![], None).await.unwrap();
        if let Some(row) = rows.rows.first() {
            return serde_json::to_value(row)
                .unwrap()
                .as_array()
                .unwrap()
                .clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no row for: {sql}");
}

/// The effort-review example over a real oxplow: an effort with a work
/// item finishes, its review is asked for by the extension's effect, the
/// model reads the effort's evidence (the test run the agent reported and
/// the diff coverage) through the extension's tool, and the review is
/// commented on the work item and published in the exported model. A
/// second review, cancelled through oxplow's own Jobs command while the
/// model writes, keeps what it had and says it was cancelled.
#[tokio::test(flavor = "multi_thread")]
async fn the_effort_review_example_reviews_evidence_and_can_be_cancelled() {
    use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/effort-review");
    copy_dir(&example, &root.join(".oxplow/extensions/effort-review"));

    let tested = test_extension(root, "effort-review", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());

    let svc = common::booted(root).await;
    // The model: a call to the evidence tool, the review, then a slow one
    // to cancel.
    let replies: Vec<oxplow_ai::fixture::FixtureReply> = serde_json::from_value(json!([
        { "toolCalls": [{ "name": "effort_evidence", "arguments": { "effort": "effort:eff1" } }] },
        "The agent's 12 tests pass; diff coverage is 80%, leaving src/a.rs:3 untested.",
        { "text": "A long review that is still being written when it is cancelled, word by word.", "chunkDelayMs": 100 },
    ]))
    .unwrap();
    let model = std::sync::Arc::new(oxplow_ai::fixture::FixtureModel::new(replies));
    svc.ai.providers().register_host(model.clone());
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "fixture".into(),
                kind: oxplow_ai::fixture::KIND.into(),
                base_url: None,
            },
            None,
        )
        .unwrap();
    svc.ai
        .set_role(
            &Role::parse("effort-review/review").expect("a profile role"),
            Some(RoleBinding::new("fixture", "fixture")),
        )
        .unwrap();
    let ext = svc.extension_catalog.named(root, "effort-review").unwrap();
    for decl in &ext.effects {
        oxplow_app::effects::start_at_head(&svc.db, &decl.name())
            .await
            .unwrap();
    }
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("it activates");
    oxplow_app::effect_triggers::register(&svc);
    // The pump delivers events to the effort lifecycle and the effects, as
    // in the app.
    svc.event_pump.clone().spawn();

    // An effort on a work item: the agent ran the tests, then it finished.
    svc.streams.ensure_primary().await.unwrap();
    let item = as_person(
        &svc,
        "oxplow.work_item.create",
        json!({ "title": "Parse blocks" }),
    )
    .await;
    let item = item["ref"].as_str().unwrap().to_string();
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thread:thr1", "work_item": item }),
    )
    .await;
    svc.commands
        .run(
            &oxplow_domain::Actor::Agent {
                session_id: None,
                thread_id: Some(oxplow_domain::ThreadId::new(1)),
                stream_id: None,
            },
            "oxplow.test.record_run",
            json!({ "command": "cargo test", "passed": 12, "failed": 0, "total": 12 }),
            false,
        )
        .await
        .unwrap();
    as_person(
        &svc,
        "oxplow.effort.close",
        json!({ "effort": "effort:eff1" }),
    )
    .await;

    // Its effect asked for the review; the scheduler isn't running yet, so
    // it waits while the coverage report arrives.
    let job = until_row(
        &svc,
        "SELECT id FROM v_job_run WHERE intent_key = 'review:effort:eff1'",
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string();
    let store = &svc.effort_evidence_store;
    let mut observations = store.list_observations(1, None).await.unwrap();
    observations.push(oxplow_db::EffortObservation {
        kind: "diff-coverage".into(),
        provenance: "observed".into(),
        source: "lcov".into(),
        metric_value: Some(80.0),
        payload_json: Some(r#"{"uncovered":[{"path":"src/a.rs","lines":[3]}]}"#.into()),
        local_snapshot_id: None,
        created_at: oxplow_domain::Timestamp::now(),
    });
    let sig = store.attribution_sig(1).await.unwrap();
    store
        .replace(1, Vec::new(), observations, sig)
        .await
        .unwrap();
    svc.jobs
        .spawn(svc.extension_activation.clone(), svc.commands.clone());

    // The model read the evidence: the test run and the coverage.
    let evidence = until_row(
        &svc,
        &format!(
            "SELECT data FROM v_job_run_output WHERE job_id = '{job}' AND kind = 'tool_result'"
        ),
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string();
    let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
    let bundle = &evidence["result"];
    assert_eq!(bundle["workItem"], json!(item), "{evidence}");
    assert_eq!(bundle["testRuns"][0]["passed"], json!(12), "{evidence}");
    assert!(
        bundle["observations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == json!("diff-coverage") && o["metric_value"] == json!(80.0)),
        "{evidence}"
    );

    // The review asked for its own profile, which the person bound.
    let roles = until_row(&svc, "SELECT DISTINCT role FROM v_ai_call").await;
    assert_eq!(roles, [json!("effort-review/review")]);

    // The review is commented on the work item, and published.
    let comment = until_row(
        &svc,
        "SELECT body FROM v_work_item_comment WHERE body LIKE '**Review of effort:eff1**%'",
    )
    .await;
    assert!(
        comment[0]
            .as_str()
            .unwrap()
            .contains("diff coverage is 80%"),
        "{comment:?}"
    );
    let published = until_row(
        &svc,
        "SELECT effort, work_item, state, review FROM v_effort_review_reviews",
    )
    .await;
    assert_eq!(
        published,
        [
            json!("effort:eff1"),
            json!(item),
            json!("succeeded"),
            json!("The agent's 12 tests pass; diff coverage is 80%, leaving src/a.rs:3 untested.")
        ]
    );

    // A second effort's review, cancelled from oxplow's Jobs command while
    // the model writes: it ends cancelled, and the comment keeps what it had.
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thread:thr1", "work_item": item }),
    )
    .await;
    as_person(
        &svc,
        "oxplow.effort.close",
        json!({ "effort": "effort:eff2" }),
    )
    .await;
    let second = until_row(
        &svc,
        "SELECT job_id FROM v_job_run_output o JOIN v_job_run j ON j.id = o.job_id \
          WHERE j.intent_key = 'review:effort:eff2' AND o.kind = 'text'",
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string();
    as_person(
        &svc,
        "oxplow.job_run.cancel",
        json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&second) }),
    )
    .await;
    until_row(
        &svc,
        &format!("SELECT 1 FROM v_job_run WHERE id = '{second}' AND state = 'cancelled'"),
    )
    .await;
    until_row(
        &svc,
        "SELECT body FROM v_work_item_comment WHERE body LIKE '**Review of effort:eff2**%The review was cancelled after%'",
    )
    .await;
}

/// oxplow-bundled's inferred reasoning over a real oxplow: when an effort
/// finishes, `infer-reasoning` asks the `reasoning` job on the person's
/// `oxplow-bundled/reasoning` binding, and `store-reasoning` stores the
/// model's proposals as the effort's inferred decisions and claims,
/// through core's commands. With no model bound to the profile or to
/// Main, nothing is asked and the effect's skip says where to bind one.
/// Also returns the model, to read the requests it was asked.
async fn inferred_reasoning_run(
    bound: bool,
) -> (
    std::sync::Arc<oxplow_app::Services>,
    tempfile::TempDir,
    std::sync::Arc<oxplow_ai::fixture::FixtureModel>,
) {
    use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let svc = common::booted(root).await;
    let model = std::sync::Arc::new(oxplow_ai::fixture::FixtureModel::new(
        serde_json::from_value::<Vec<oxplow_ai::fixture::FixtureReply>>(serde_json::json!([
            "```json\n{\"decisions\": [{\"question\": \"Which CSV library?\", \"choice\": \"csv crate\", \"alternatives\": [\"hand-rolled\"], \"confidence\": \"medium\"}], \"claims\": [{\"statement\": \"The export tests pass.\", \"kind\": \"tests_pass\", \"evidence_ref\": \"run:12\"}]}\n```"
        ]))
        .unwrap(),
    ));
    if bound {
        svc.ai.providers().register_host(model.clone());
        svc.ai
            .save_provider(
                ProviderConfig {
                    id: "fixture".into(),
                    kind: oxplow_ai::fixture::KIND.into(),
                    base_url: None,
                },
                None,
            )
            .unwrap();
        svc.ai
            .set_role(
                &Role::parse("oxplow-bundled/reasoning").expect("a profile role"),
                Some(RoleBinding::new("fixture", "fixture")),
            )
            .unwrap();
    }
    let ext = svc
        .extension_catalog
        .named(root, "oxplow-bundled")
        .expect("oxplow-bundled is active");
    for decl in &ext.effects {
        oxplow_app::effects::start_at_head(&svc.db, &decl.name())
            .await
            .unwrap();
    }
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("it activates");
    oxplow_app::effect_triggers::register(&svc);
    svc.event_pump.clone().spawn();
    svc.jobs
        .spawn(svc.extension_activation.clone(), svc.commands.clone());

    // An effort on a work item in which the agent edited a file, finished.
    svc.streams.ensure_primary().await.unwrap();
    let item = as_person(
        &svc,
        "oxplow.work_item.create",
        json!({ "title": "Add CSV export" }),
    )
    .await;
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thread:thr1", "work_item": item["ref"] }),
    )
    .await;
    svc.tool_call_store
        .record(oxplow_db::NewToolCall {
            thread_id: 1,
            effort_id: Some(1),
            tool: "Edit".into(),
            kind: "edit".into(),
            path: Some("src/report.rs".into()),
            detail: None,
            ok: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    // …and said how it went in its turn's final answer.
    svc.db
        .transaction(|tx| {
            tx.execute(
                "INSERT INTO agent_turn (thread_id, prompt, answer, started_at, ended_at) \
                 VALUES (1, 'Add CSV export', 'Added the export. The export tests pass (run:12).', \
                         '2000-01-01T00:00:00Z', '2100-01-01T00:00:00Z')",
                (),
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
    as_person(
        &svc,
        "oxplow.effort.close",
        json!({ "effort": "effort:eff1" }),
    )
    .await;
    (svc, dir, model)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bundled_extension_infers_an_efforts_decisions_and_claims_on_its_profile() {
    use serde_json::json;
    let (svc, _dir, model) = inferred_reasoning_run(true).await;
    let stored = until_row(
        &svc,
        "SELECT question, choice, alternatives, confidence FROM v_decision \
          WHERE effort_id = 1 AND provenance = 'inferred'",
    )
    .await;
    assert_eq!(
        stored,
        [
            json!("Which CSV library?"),
            json!("csv crate"),
            json!(r#"["hand-rolled"]"#),
            json!("medium")
        ]
    );
    // The claims its answers made are stored as unreviewed ones, citing
    // what the answer named.
    let claim = until_row(
        &svc,
        "SELECT statement, kind, evidence_ref, review FROM v_claim WHERE effort_id = 1",
    )
    .await;
    assert_eq!(
        claim,
        [
            json!("The export tests pass."),
            json!("tests_pass"),
            json!("run:12"),
            json!(null)
        ]
    );
    // The prompt carried what the agent did, its final answer included;
    // the call was on the profile.
    let calls = until_row(&svc, "SELECT role, count(*) FROM v_ai_call GROUP BY role").await;
    assert_eq!(calls, [json!("oxplow-bundled/reasoning"), json!(1)]);
    let asked = serde_json::to_string(&model.asked_typed()).unwrap();
    assert!(
        asked.contains("Agent: Added the export. The export tests pass (run:12)."),
        "{asked}"
    );
    let jobs = until_row(
        &svc,
        "SELECT count(*) FROM v_job_run WHERE intent_key = 'reasoning:effort:eff1'",
    )
    .await;
    assert_eq!(jobs, [json!(1)]);
    // A person's Infer Again is a new job (the effect's intent is its own);
    // its unchanged request reads the recorded answer.
    as_person(
        &svc,
        "oxplow.effort.infer_reasoning",
        json!({ "effort": "effort:eff1", "again": true }),
    )
    .await;
    until_row(
        &svc,
        "SELECT 1 FROM v_job_run WHERE intent_key = 'reasoning:effort:eff1@2' AND state = 'succeeded'",
    )
    .await;
    let calls = until_row(&svc, "SELECT count(*) FROM v_ai_call").await;
    assert_eq!(calls, [json!(1)], "the second job read the recorded answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_model_inferring_reasoning_skips_and_says_where_to_bind_one() {
    let (svc, _dir, _model) = inferred_reasoning_run(false).await;
    let skipped = until_row(
        &svc,
        "SELECT reason FROM v_effect_run \
          WHERE effect = 'oxplow-bundled/infer-reasoning' AND state = 'skipped'",
    )
    .await;
    let reason = skipped[0].as_str().unwrap();
    assert!(
        reason.contains("oxplow-bundled/reasoning") && reason.contains("Settings → AI"),
        "{reason}"
    );
    let jobs = svc
        .sql
        .query_sql("SELECT count(*) FROM v_job_run", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&jobs.rows).unwrap(),
        serde_json::json!([[0]])
    );
}

/// The session-notes example adds a section to a core page: it checks
/// clean, its mount is the New Session page's sections slot placed after
/// `start`, and its example runs.
#[tokio::test(flavor = "multi_thread")]
async fn the_session_notes_example_adds_a_section_to_the_new_session_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
    copy_dir(
        &examples.join("session-notes"),
        &root.join(".oxplow/extensions/session-notes"),
    );
    let report = check(
        root,
        "session-notes",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "session-notes", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    assert!(
        tested.ran.contains(&"example thread-notes".to_string()),
        "{:?}",
        tested.ran
    );
    let ext = ExtensionCatalog::new()
        .named(root, "session-notes")
        .unwrap();
    let mount = &ext.ui.slots[0];
    assert_eq!(mount.slot, "new-session.sections");
    assert_eq!(
        mount.at,
        Some(oxplow_app::extensions::SlotAt::After {
            after: "start".into()
        })
    );
    assert!(ext.warnings.is_empty(), "{:?}", ext.warnings);
}

/// The commit-history example is a page of typed params and a row that
/// selects: it checks clean, its params keep their types and places, its
/// commits column selects the page's `commit`, and its examples run.
#[tokio::test(flavor = "multi_thread")]
async fn the_commit_history_example_narrows_with_typed_params_and_selects_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
    copy_dir(
        &examples.join("commit-history"),
        &root.join(".oxplow/extensions/commit-history"),
    );
    let report = check(
        root,
        "commit-history",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "commit-history", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    for name in ["example commits", "example files"] {
        assert!(tested.ran.contains(&name.to_string()), "{:?}", tested.ran);
    }
    let ext = ExtensionCatalog::new()
        .named(root, "commit-history")
        .unwrap();
    assert!(ext.errors.is_empty(), "{:?}", ext.errors);
    let page = ext.lenses.iter().find(|l| l.slug == "history").unwrap();
    use oxplow_app::extensions::{ParamPlace, ParamType};
    let params: Vec<_> = page
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.kind, p.place))
        .collect();
    assert_eq!(
        params,
        vec![
            ("range", Some(ParamType::DateRange), ParamPlace::Toolbar),
            ("q", Some(ParamType::Search), ParamPlace::Toolbar),
            ("merges", Some(ParamType::Bool), ParamPlace::Toolbar),
            ("commit", Some(ParamType::Text), ParamPlace::Rail),
        ]
    );
    let commits = ext.lenses.iter().find(|l| l.slug == "commits").unwrap();
    assert_eq!(commits.columns[0].select.as_deref(), Some("commit"));
}

/// Both examples copied into one project.
async fn review_project(with_producer: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
    copy_dir(
        &examples.join("review-badges"),
        &root.join(".oxplow/extensions/review-badges"),
    );
    if with_producer {
        copy_dir(
            &examples.join("effort-review"),
            &root.join(".oxplow/extensions/effort-review"),
        );
    }
    dir
}

/// The review-badges example reads effort-review's exported `reviews`
/// model, optionally: with the producer present its lenses and slot load;
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
    assert_eq!(tested.errors, Vec::<String>::new());
    assert_eq!(
        tested.imports,
        ["`reviews` → model:effort-review/reviews v2: available"]
    );
    for id in [
        "lens:review-badges/review-verdicts",
        "lens:review-badges/item-review",
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

    // The producer disabled.
    let dir = review_project(true).await;
    let root = dir.path();
    std::fs::create_dir_all(root.join(".oxplow")).unwrap();
    std::fs::write(
        root.join(".oxplow/personal.yaml"),
        "disableExtensions: [effort-review]\n",
    )
    .unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    // The lenses and the slot are withheld, named in the one warning for
    // the import; the fixtures that test them are skipped, saying why.
    assert_eq!(
        tested.imports,
        ["`reviews` → model:effort-review/reviews v2: unavailable (effort-review is disabled)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);
    let import_warnings: Vec<_> = tested
        .warnings
        .iter()
        .filter(|w| w.contains("import `reviews`") && !w.contains("fixtures/"))
        .collect();
    assert_eq!(import_warnings.len(), 1, "{:?}", tested.warnings);
    let warning = import_warnings[0];
    assert!(
        warning.contains(
            "optional import `reviews` for `model:effort-review/reviews` v2 is unavailable: \
             effort-review is disabled; withheld: "
        ),
        "{warning}"
    );
    for id in [
        "`lens:review-badges/item-review`",
        "`lens:review-badges/review-verdicts`",
        "`slot:review-badges/work_item.detail.body-0`",
    ] {
        assert!(warning.contains(id), "{id}: {warning}");
    }
    assert!(
        !tested
            .warnings
            .iter()
            .any(|w| w.contains("because an import")),
        "{:?}",
        tested.warnings
    );
    assert_eq!(tested.errors, Vec::<String>::new());
    for (example, lens) in [
        ("review-verdicts", "review-verdicts"),
        ("item-review", "item-review"),
    ] {
        assert!(
            tested.warnings.iter().any(|w| w.contains(&format!(
                "fixtures/{example}.yaml:1: example `{example}` isn't run: \
                 `lens:review-badges/{lens}` is withheld because import `reviews` is \
                 unavailable (effort-review is disabled)"
            ))),
            "{example}: {:?}",
            tested.warnings
        );
    }
    let svc = common::booted(root).await;
    let consumer = svc.extension_catalog.named(root, "review-badges").unwrap();
    assert!(consumer.ui.slots.is_empty(), "{:?}", consumer.ui.slots);

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
            "contribution: model:effort-review/reviews, version: 2,",
            "contribution: model:effort-review/reviews, version: 3,",
        );
    assert_ne!(bumped, text);
    std::fs::write(&manifest, bumped).unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → model:effort-review/reviews v2: unavailable (effort-review exports v3)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);

    // The producer missing: unavailable, and no error from loading it.
    let dir = review_project(false).await;
    let root = dir.path();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → model:effort-review/reviews v2: unavailable (effort-review or its contribution isn't here)"]
    );
    assert_eq!(tested.errors, Vec::<String>::new());
}

/// With both examples running in one oxplow, a review effort-review's job
/// wrote shows up in review-badges' lenses: the list, and the task page's
/// slot lens for that item and none other.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_effort_review_wrote_shows_in_review_badges() {
    use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
    use serde_json::json;

    let dir = review_project(true).await;
    let root = dir.path();
    let svc = common::booted(root).await;
    let replies: Vec<oxplow_ai::fixture::FixtureReply> = serde_json::from_value(json!([
        "The tests pass and coverage is fine; one untested branch remains in the parser.",
    ]))
    .unwrap();
    let model = std::sync::Arc::new(oxplow_ai::fixture::FixtureModel::new(replies));
    svc.ai.providers().register_host(model);
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "fixture".into(),
                kind: oxplow_ai::fixture::KIND.into(),
                base_url: None,
            },
            None,
        )
        .unwrap();
    svc.ai
        .set_role(
            &Role::SUMMARIZE,
            Some(RoleBinding::new("fixture", "fixture")),
        )
        .unwrap();
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("it activates");
    let consumer = svc.extension_catalog.named(root, "review-badges").unwrap();
    assert_eq!(consumer.ui.slots.len(), 1, "{:?}", consumer.ui.slots);

    // An effort on a work item, then the review of it asked for.
    svc.streams.ensure_primary().await.unwrap();
    let item = as_person(
        &svc,
        "oxplow.work_item.create",
        json!({ "title": "Parse blocks" }),
    )
    .await["ref"]
        .as_str()
        .unwrap()
        .to_string();
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thread:thr1", "work_item": item }),
    )
    .await;
    as_person(
        &svc,
        "oxplow.effort.close",
        json!({ "effort": "effort:eff1" }),
    )
    .await;
    as_person(
        &svc,
        "effort_review.review.ask",
        json!({
            "job": "review",
            "intentKey": "review:effort:eff1",
            "request": {
                "schemaVersion": 1,
                "role": "summarize",
                "messages": [{ "role": "user", "parts": [{ "type": "text", "text": "Review effort:eff1." }] }],
            },
        }),
    )
    .await;
    svc.jobs
        .spawn(svc.extension_activation.clone(), svc.commands.clone());
    until_row(
        &svc,
        "SELECT 1 FROM v_effort_review_reviews WHERE state = 'succeeded'",
    )
    .await;

    let lens = |id: &'static str, params: Vec<(&'static str, oxplow_db::SqlCell)>| {
        let svc = svc.clone();
        async move {
            let run = oxplow_app::extensions::run_lens(
                &svc.sql,
                &svc.extension_catalog,
                root,
                id,
                params
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                &oxplow_app::extensions::LensContext::default(),
            )
            .await
            .unwrap();
            serde_json::to_value(&run.result.rows).unwrap()
        }
    };
    let list = lens("review-badges/review-verdicts", vec![]).await;
    assert_eq!(list[0][0], json!(item), "{list}");
    assert_eq!(list[0][1], json!("effort:eff1"));
    assert_eq!(list[0][2], json!("succeeded"));
    assert!(list[0][4]
        .as_str()
        .unwrap()
        .starts_with("The tests pass and coverage is fine"));
    let here = lens(
        "review-badges/item-review",
        vec![("ref", oxplow_db::SqlCell::Text(item.clone()))],
    )
    .await;
    assert_eq!(here[0][0], json!("succeeded"), "{here}");
    let elsewhere = lens(
        "review-badges/item-review",
        vec![(
            "ref",
            oxplow_db::SqlCell::Text("work_item:oxplow:tsk99".into()),
        )],
    )
    .await;
    assert_eq!(elsewhere, json!([]));
}

/// The schema-review example is a review policy of its own, built on public
/// contracts only: its collector's facts (the DDL in each migration file,
/// sliced by its own dimension) are joined with core's evidence — the
/// effort's files, its latest test run and the functions it added — into
/// a verdict that replaces oxplow-bundled's on the effort's page.
#[tokio::test(flavor = "multi_thread")]
async fn the_schema_review_example_joins_its_facts_with_core_evidence() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/schema-review");
    copy_dir(&example, &root.join(".oxplow/extensions/schema-review"));
    let report = check(
        root,
        "schema-review",
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    let tested = test_extension(root, "schema-review", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());
    assert!(
        tested.ran.contains(&"example ddl-counted".to_string()),
        "{:?}",
        tested.ran
    );

    // A real oxplow: an effort that adds a migration and a function.
    let svc = common::booted(root).await;
    // Both review policies offer an effort's verdict; the person chooses.
    let offering: Vec<String> = svc
        .extension_catalog
        .get(root)
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.ui.replacements.iter())
        .filter(|r| r.target == "effort.verdict")
        .map(|r| r.extension.clone())
        .collect();
    assert_eq!(offering, ["oxplow-bundled", "schema-review"]);

    // The pump delivers events to the effort lifecycle, as in the app: it
    // pins the effort's start and end snapshots.
    svc.event_pump.clone().spawn();
    svc.streams.ensure_primary().await.unwrap();
    let stream = svc.streams.list_streams().await.unwrap()[0].id;
    let capture = svc.snapshot_captures.get(&stream).unwrap();
    // A snapshot of what the worktree holds now (the app's watcher marks
    // what changed; here the tree is diffed).
    let take = || async {
        capture.enqueue_startup_diff().await.unwrap();
        capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Manual)
            .await
            .unwrap();
    };
    take().await;
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thread:thr1" }),
    )
    .await;
    until_row(
        &svc,
        "SELECT 1 FROM v_effort WHERE id = 1 AND start_snapshot_id IS NOT NULL",
    )
    .await;
    let thread = oxplow_domain::ThreadId::new(1);
    let write = |path: &str, text: &str| {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write(
        "migrations/0001_users.sql",
        "CREATE TABLE users (id INT);\nALTER TABLE accounts ADD owner INT;\n",
    );
    // A function with 16 branches: complexity 17.
    let branches: String = (0..16)
        .map(|i| format!("    if x == {i} {{ return {i}; }}\n"))
        .collect();
    write(
        "src/lib.rs",
        &format!("pub fn pick(x: i32) -> i32 {{\n{branches}    0\n}}\n"),
    );
    for path in ["migrations/0001_users.sql", "src/lib.rs"] {
        assert!(svc
            .efforts
            .claim_open_effort_file(&thread, path, Some(root))
            .await
            .unwrap());
    }
    // The agent reports its tests passed: reported, not observed.
    svc.commands
        .run(
            &oxplow_domain::Actor::Agent {
                session_id: None,
                thread_id: Some(thread),
                stream_id: None,
            },
            "oxplow.test.record_run",
            json!({ "command": "cargo test", "passed": 3, "failed": 0, "total": 3 }),
            false,
        )
        .await
        .unwrap();
    // Closing takes the effort's end snapshot of what changed.
    capture.enqueue_startup_diff().await.unwrap();
    as_person(
        &svc,
        "oxplow.effort.close",
        json!({ "effort": "effort:eff1" }),
    )
    .await;
    until_row(
        &svc,
        "SELECT 1 FROM v_effort WHERE id = 1 AND end_snapshot_id IS NOT NULL",
    )
    .await;
    oxplow_app::change_analysis::ensure_change(
        &svc,
        oxplow_app::change_analysis::ChangeTarget::Effort {
            effort_id: "eff1".into(),
        },
    )
    .await
    .unwrap();
    let collect = || async {
        svc.metrics
            .run_collector_by_key(
                "schema-review",
                "ddl",
                Some(stream),
                &oxplow_domain::run::RunContext::core(
                    oxplow_domain::Actor::Human,
                    oxplow_domain::run::Mode::External,
                ),
            )
            .await
            .unwrap()
    };
    collect().await;
    let verdict = || async {
        let rows = svc
            .sql
            .query_sql(
                "SELECT verdict, reasons, statements FROM v_schema_review_verdict WHERE effort_id = 1",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&rows.rows).unwrap()
    };
    assert_eq!(
        verdict().await,
        json!([[
            "changes requested",
            "its last test run was only reported; a new function of complexity 17",
            2.0
        ]])
    );

    // The migration drops a table now: the next snapshot's facts say so.
    write(
        "migrations/0001_users.sql",
        "CREATE TABLE users (id INT);\nDROP TABLE accounts;\n",
    );
    take().await;
    collect().await;
    assert_eq!(
        verdict().await[0][1],
        json!(
            "drops a table; its last test run was only reported; a new function of complexity 17"
        )
    );

    // Its verdict lens, as the effort's page shows it in place of
    // oxplow-bundled's.
    let layer = oxplow_app::sql_gateway::SqlGateway::new(svc.db.clone());
    let run = oxplow_app::extensions::run_lens(
        &layer,
        &svc.extension_catalog,
        root,
        "schema-review/effort-verdict",
        [("effort_id".to_string(), oxplow_db::SqlCell::Int(1))]
            .into_iter()
            .collect(),
        &oxplow_app::extensions::LensContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&run.result.rows).unwrap(),
        json!([
            ["Schema review: changes requested", "danger"],
            ["• drops a table", "danger"],
            ["• its last test run was only reported", "danger"],
            ["• a new function of complexity 17", "danger"],
        ])
    );
}
