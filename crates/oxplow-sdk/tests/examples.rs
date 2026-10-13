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
    use oxplow_app::exec_consent::{approve_every, ProgramKind};
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/effort-review");
    copy_dir(&example, &root.join("oxplow/extensions/effort-review"));

    let tested = test_extension(root, "effort-review", false).await.unwrap();
    assert_eq!(tested.errors, Vec::<String>::new());

    let svc = oxplow_app::Services::in_memory(root).await.unwrap();
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
            Role::Summarize,
            Some(RoleBinding {
                provider: "fixture".into(),
                model: "fixture".into(),
            }),
        )
        .unwrap();
    // A person approves the job and the two effects.
    approve_every(&svc, ProgramKind::Job).unwrap();
    approve_every(&svc, ProgramKind::Effect).unwrap();
    let ext = svc.extension_catalog.named(root, "effort-review").unwrap();
    for decl in &ext.effects {
        oxplow_app::effects::approved(&svc.db, &decl.name())
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
        "SELECT id FROM v_job WHERE intent_key = 'review:effort:eff1'",
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
        &format!("SELECT data FROM v_job_output WHERE job_id = '{job}' AND kind = 'tool_result'"),
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
        "SELECT job_id FROM v_job_output o JOIN v_job j ON j.id = o.job_id \
          WHERE j.intent_key = 'review:effort:eff2' AND o.kind = 'text'",
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string();
    as_person(&svc, "oxplow.job.cancel", json!({ "id": second })).await;
    until_row(
        &svc,
        &format!("SELECT 1 FROM v_job WHERE id = '{second}' AND state = 'cancelled'"),
    )
    .await;
    until_row(
        &svc,
        "SELECT body FROM v_work_item_comment WHERE body LIKE '**Review of effort:eff2**%The review was cancelled after%'",
    )
    .await;
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
        &root.join("oxplow/extensions/review-badges"),
    );
    if with_producer {
        copy_dir(
            &examples.join("effort-review"),
            &root.join("oxplow/extensions/effort-review"),
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
        ["`reviews` → effort-review/model/reviews v1: available"]
    );
    for id in [
        "review-badges/lens/review-verdicts",
        "review-badges/lens/item-review",
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
        root.join(".oxplow/project.yaml"),
        "extensions:\n  disabled: [effort-review]\n",
    )
    .unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    // The lenses and the slot are withheld, named in the one warning for
    // the import; the fixtures that test them are skipped, saying why.
    assert_eq!(
        tested.imports,
        ["`reviews` → effort-review/model/reviews v1: unavailable (effort-review is disabled)"]
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
            "optional import `reviews` for `effort-review/model/reviews` v1 is unavailable: \
             effort-review is disabled; withheld: "
        ),
        "{warning}"
    );
    for id in [
        "`review-badges/lens/item-review`",
        "`review-badges/lens/review-verdicts`",
        "`review-badges/slot/work_item.detail.body-0`",
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
                 `review-badges/lens/{lens}` is withheld because import `reviews` is \
                 unavailable (effort-review is disabled)"
            ))),
            "{example}: {:?}",
            tested.warnings
        );
    }
    let svc = oxplow_app::Services::in_memory(root).await.unwrap();
    let consumer = svc.extension_catalog.named(root, "review-badges").unwrap();
    assert!(consumer.ui.slots.is_empty(), "{:?}", consumer.ui.slots);

    // The producer exports another version.
    let dir = review_project(true).await;
    let root = dir.path();
    let manifest = root.join("oxplow/extensions/effort-review/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let bumped = text
        .replace(
            "    version: 1\n    key: [job]",
            "    version: 2\n    key: [job]",
        )
        .replace(
            "contribution: effort-review/model/reviews, version: 1,",
            "contribution: effort-review/model/reviews, version: 2,",
        );
    assert_ne!(bumped, text);
    std::fs::write(&manifest, bumped).unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → effort-review/model/reviews v1: unavailable (effort-review exports v2)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);

    // The producer missing: unavailable, and no error from loading it.
    let dir = review_project(false).await;
    let root = dir.path();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → effort-review/model/reviews v1: unavailable (effort-review or its contribution isn't here)"]
    );
    assert_eq!(tested.errors, Vec::<String>::new());
}

/// With both examples running in one oxplow, a review effort-review's job
/// wrote shows up in review-badges' lenses: the list, and the task page's
/// slot lens for that item and none other.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_effort_review_wrote_shows_in_review_badges() {
    use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
    use oxplow_app::exec_consent::{approve_every, ProgramKind};
    use serde_json::json;

    let dir = review_project(true).await;
    let root = dir.path();
    let svc = oxplow_app::Services::in_memory(root).await.unwrap();
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
            Role::Summarize,
            Some(RoleBinding {
                provider: "fixture".into(),
                model: "fixture".into(),
            }),
        )
        .unwrap();
    approve_every(&svc, ProgramKind::Job).unwrap();
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
