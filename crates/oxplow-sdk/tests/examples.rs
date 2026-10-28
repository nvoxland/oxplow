//! P10 (K3): the documented github example is the first-party use that
//! made `ref_kinds` stable. It checks and tests clean, and its
//! pull-request kind works in a real oxplow: `[[pr:12]]` names
//! `github_pr:12`, which resolves to the pull request's title and opens
//! its `pr` page.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use oxplow_sdk::{
    check, extension_test::test_extension, render_findings, ExtensionCatalog, Format,
};

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

/// Page `id` (`<extension>/<slug>`) run as the viewer, given `params`, over
/// `svc`'s data.
async fn run_page(
    svc: &oxplow_app::Services,
    id: &str,
    params: Vec<(&str, oxplow_db::SqlCell)>,
) -> oxplow_app::pages::runner::PageRun {
    use oxplow_app::pages::runner::{PageRunner, RunRequest, RunTarget};
    let run = PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(id.to_string()),
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            ctx: oxplow_app::extensions::ViewContext::default(),
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .unwrap();
    for (name, loader) in &run.loaders {
        assert_eq!(loader.error, None, "loader `{name}` of `{id}`");
    }
    run
}

/// The rows of page `run`'s loader `name`, as JSON.
fn loader_rows(
    run: &oxplow_app::pages::runner::PageRun,
    name: &str,
) -> Vec<Vec<serde_json::Value>> {
    run.loaders[name]
        .result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| serde_json::to_value(c).unwrap())
                .collect()
        })
        .collect()
}

/// The triage example uses all three host services in one fact collector:
/// the tree it implies, the semantic layer and a model it declares. It
/// checks clean and lists them; `test` runs its fixtures; and in a real
/// oxplow one snapshot's TODO becomes a
/// classified fact beside the open work, every service decided on its
/// run.
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
    assert_eq!(tested.errors, Vec::<String>::new());
    assert!(
        !tested.warnings.iter().any(|w| w.contains("calls a model")),
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
            "```json\n{\"decisions\": [{\"question\": \"Which CSV library?\", \"choice\": \"csv crate\", \"alternatives\": [\"hand-rolled\"], \"confidence\": \"medium\"}], \"claims\": [{\"statement\": \"The export tests pass.\", \"kind\": \"tests_pass\", \"evidence\": \"oxplow:oxplow/run?id=12\"}]}\n```"
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
        json!({ "thread": "thr1", "work_item": item["uri"] }),
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
    as_person(&svc, "oxplow.effort.close", json!({ "effort": "eff1" })).await;
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
        "SELECT statement, kind, evidence, review FROM v_claim WHERE effort_id = 1",
    )
    .await;
    assert_eq!(
        claim,
        [
            json!("The export tests pass."),
            json!("tests_pass"),
            json!("oxplow:oxplow/run?id=12"),
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
        "SELECT count(*) FROM v_job_run WHERE intent_key = 'reasoning:eff1'",
    )
    .await;
    assert_eq!(jobs, [json!(1)]);
    // A person's Infer Again is a new job (the effect's intent is its own);
    // its unchanged request reads the recorded answer.
    as_person(
        &svc,
        "oxplow.effort.infer_reasoning",
        json!({ "effort": "eff1", "again": true }),
    )
    .await;
    until_row(
        &svc,
        "SELECT 1 FROM v_job_run WHERE intent_key = 'reasoning:eff1@2' AND state = 'succeeded'",
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
    assert_eq!(tested.errors, Vec::<String>::new());
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/model?model=effort-review/reviews v2: available"]
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
        ["`reviews` → oxplow:oxplow/model?model=effort-review/reviews v2: unavailable (effort-review or its contribution isn't here)"]
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
            "optional import `reviews` for `oxplow:oxplow/model?model=effort-review/reviews` v2 is unavailable: \
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
            .warnings
            .iter()
            .any(|w| w.contains("because an import")),
        "{:?}",
        tested.warnings
    );
    assert_eq!(tested.errors, Vec::<String>::new());
    for (example, page) in [
        ("review-verdicts", "review-verdicts"),
        ("item-review", "item-review"),
    ] {
        assert!(
            tested.warnings.iter().any(|w| w.contains(&format!(
                "fixtures/{example}.yaml:1: example `{example}` isn't run: \
                 `oxplow:oxplow/page?page=review-badges/{page}` is withheld because import `reviews` is \
                 unavailable (effort-review or its contribution isn't here)"
            ))),
            "{example}: {:?}",
            tested.warnings
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
        ["`reviews` → oxplow:oxplow/model?model=effort-review/reviews v2: available"]
    );
    let svc = common::booted(root).await;
    let consumer = svc.extension_catalog.named(root, "review-badges").unwrap();
    assert!(
        consumer.ui.page_slots.is_empty(),
        "{:?}",
        consumer.ui.page_slots
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
            "contribution: oxplow:oxplow/model?model=effort-review/reviews, version: 2,",
            "contribution: oxplow:oxplow/model?model=effort-review/reviews, version: 3,",
        );
    assert_ne!(bumped, text);
    std::fs::write(&manifest, bumped).unwrap();
    let tested = test_extension(root, "review-badges", false).await.unwrap();
    assert_eq!(
        tested.imports,
        ["`reviews` → oxplow:oxplow/model?model=effort-review/reviews v2: unavailable (effort-review exports v3)"]
    );
    assert!(tested.contributes.is_empty(), "{:?}", tested.contributes);
}

/// With both examples running in one oxplow, a review effort-review's job
/// wrote shows up in review-badges' pages: the list, and the task page's
/// slot page for that item and none other.
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
    assert_eq!(
        consumer.ui.page_slots.len(),
        1,
        "{:?}",
        consumer.ui.page_slots
    );

    // An effort on a work item, then the review of it asked for.
    svc.streams.ensure_primary().await.unwrap();
    let item = as_person(
        &svc,
        "oxplow.work_item.create",
        json!({ "title": "Parse blocks" }),
    )
    .await["uri"]
        .as_str()
        .unwrap()
        .to_string();
    as_person(
        &svc,
        "oxplow.effort.open",
        json!({ "thread": "thr1", "work_item": item }),
    )
    .await;
    as_person(&svc, "oxplow.effort.close", json!({ "effort": "eff1" })).await;
    as_person(
        &svc,
        "effort_review.review.ask",
        json!({
            "job": "review",
            "intentKey": "review:eff1",
            "request": {
                "schemaVersion": 1,
                "role": "summarize",
                "messages": [{ "role": "user", "parts": [{ "type": "text", "text": "Review eff1." }] }],
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

    let page = |id: &'static str,
                loader: &'static str,
                params: Vec<(&'static str, oxplow_db::SqlCell)>| {
        let svc = svc.clone();
        async move {
            let run = run_page(&svc, id, params).await;
            serde_json::to_value(&run.loaders[loader].result.rows).unwrap()
        }
    };
    let list = page("review-badges/review-verdicts", "review_verdicts", vec![]).await;
    assert_eq!(list[0][0], json!(item), "{list}");
    assert_eq!(list[0][1], json!("oxplow:oxplow/effort?id=eff1"));
    assert_eq!(list[0][2], json!("succeeded"));
    assert!(list[0][4]
        .as_str()
        .unwrap()
        .starts_with("The tests pass and coverage is fine"));
    let here = page(
        "review-badges/item-review",
        "item_review",
        vec![("uri", oxplow_db::SqlCell::Text(item.clone()))],
    )
    .await;
    assert_eq!(here[0][0], json!("succeeded"), "{here}");
    let elsewhere = page(
        "review-badges/item-review",
        "item_review",
        vec![(
            "uri",
            oxplow_db::SqlCell::Text("oxplow:oxplow/work_item?id=tsk99&provider=oxplow".into()),
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
    as_person(&svc, "oxplow.effort.open", json!({ "thread": "thr1" })).await;
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
    as_person(&svc, "oxplow.effort.close", json!({ "effort": "eff1" })).await;
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

    // Its verdict page, as the effort's page shows it in place of
    // oxplow-bundled's.
    let run = run_page(
        &svc,
        "schema-review/effort-verdict",
        vec![("effort_id", oxplow_db::SqlCell::Int(1))],
    )
    .await;
    assert_eq!(
        serde_json::to_value(&run.loaders["effort_verdict"].result.rows).unwrap(),
        json!([
            ["Schema review: changes requested", "danger"],
            ["• drops a table", "danger"],
            ["• its last test run was only reported", "danger"],
            ["• a new function of complexity 17", "danger"],
        ])
    );
}
