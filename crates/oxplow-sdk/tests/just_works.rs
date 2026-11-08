//! P7.C6: "just works", deterministically. For each kind `oxplow extension
//! new` makes: a temp project → scaffold → `check` (clean, no warnings) →
//! `extension test` (clean) → it loads in a real oxplow and does its job —
//! a page's row action binds, a collector's entity and dataset publish, a
//! command runs through the bus. A provider's stub is red until a real
//! program stands behind it (the fake).

#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};

use oxplow_app::extension::extension_catalog::ExtensionCatalog;
use oxplow_sdk::extension_test::{test_extension, test_extension_in};
use oxplow_sdk::{check, render_findings, scaffold, Format, Kind};

async fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    oxplow_app::vcs::GitProvider
        .init_repository(dir.path())
        .await
        .unwrap();
    dir
}

/// Scaffold `kind` as `name`; `check` is clean with no warnings.
async fn scaffolded(root: &Path, kind: &str, name: &str) {
    let kind = Kind::parse(kind).unwrap_or_else(|| panic!("`extension new {kind}`"));
    scaffold(root, kind, name, Some("oxplow:oxplow/effort?id=eff1")).unwrap();
    let report = check(root, name, &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    assert_eq!(report.warning_lines(), Vec::<String>::new(), "{name}");
}

/// `extension test` is clean; what ran.
async fn tested(root: &Path, name: &str) -> Vec<String> {
    let report = test_extension(root, name, false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new(), "{name}");
    assert_eq!(report.warning_lines(), Vec::<String>::new(), "{name}");
    report.ran
}

/// A real oxplow over the project, as at boot.
async fn booted(root: &Path) -> std::sync::Arc<oxplow_app::Services> {
    common::booted(root).await
}

fn loaded(
    svc: &oxplow_app::Services,
    root: &Path,
    name: &str,
) -> oxplow_app::extension::extensions::Extension {
    let ext = svc
        .extension_catalog
        .get(root)
        .iter()
        .find(|e| e.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} loads"));
    assert_eq!(
        ext.errors,
        Vec::<oxplow_domain::diagnostic::Diagnostic>::new(),
        "{name}"
    );
    ext
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_page_checks_tests_and_its_row_action_binds() {
    let dir = project().await;
    scaffolded(dir.path(), "page", "open-work").await;
    assert!(tested(dir.path(), "open-work")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "open-work");
    let page = &ext.page_files[0];
    let action = page
        .nodes()
        .into_iter()
        .find_map(|n| match &n.kind {
            oxplow_app::pages::schema::NodeKind::Viz { config, .. } => {
                config.actions.iter().find(|a| a.row).cloned()
            }
            _ => None,
        })
        .expect("a row action");
    let command = action.command.as_deref().expect("a command action");
    assert!(svc.commands.spec(command).is_some(), "{command}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_extension_checks_and_loads() {
    let dir = project().await;
    scaffolded(dir.path(), "extension", "my-notes").await;
    let report = test_extension(dir.path(), "my-notes", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    let svc = booted(dir.path()).await;
    loaded(&svc, dir.path(), "my-notes");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_collector_checks_before_its_first_sync_and_runs() {
    let dir = project().await;
    scaffolded(dir.path(), "collector", "open-items").await;
    assert!(tested(dir.path(), "open-items")
        .await
        .contains(&"example basic".to_string()));
    // The extension's dataset reads its collector's entity: activation
    // creates it empty, so the dataset compiles before the first run.
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "open-items");
    let collector = &ext.collectors[0];
    // It runs for real: its entity, then its dataset, publish.
    svc.commands
        .run(
            &oxplow_domain::Actor::Human,
            "oxplow.collector.sync",
            serde_json::json!({ "collector": format!("open_items/{}", collector.id).to_string() }),
            true,
        )
        .await
        .unwrap();
    svc.extension_activation
        .activate(svc.clone())
        .await
        .unwrap();
    let page = &ext.page_files[0];
    let run = oxplow_app::pages::runner::PageRunner::of(&svc)
        .run(oxplow_app::pages::runner::RunRequest {
            target: oxplow_app::pages::runner::RunTarget::Page(page.id.clone()),
            params: Default::default(),
            ctx: Default::default(),
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .unwrap();
    let items = &run.loaders["items"];
    assert_eq!(items.error, None);
    assert!(
        items.result.columns.iter().any(|c| c == "title"),
        "{:?}",
        items.result.columns
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_metric_checks_tests_and_declares_its_blocks() {
    let dir = project().await;
    scaffolded(dir.path(), "metric", "complex-fns").await;
    // Its fixture runs the collector over a corpus and expects the facts.
    assert!(tested(dir.path(), "complex-fns")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "complex-fns");

    // One per-path measure on files, namespaced under the extension.
    let [measure] = ext.measures.as_slice() else {
        panic!("one measure: {:?}", ext.measures)
    };
    let key = measure.key.as_deref().unwrap();
    assert!(key.starts_with("complex_fns."), "{key}");
    assert_eq!(measure.subject_kind.as_deref(), Some("file"));
    assert_eq!(measure.capture_scope.as_deref(), Some("per-path"));
    assert_eq!(measure.temporal_semantics.as_deref(), Some("semi-additive"));
    // The area dimension the collector tags its facts with, declared.
    let [dimension] = ext.dimensions.as_slice() else {
        panic!("one dimension: {:?}", ext.dimensions)
    };
    assert_eq!(dimension.key.as_deref(), Some("complex_fns.area"));
    // The headline metric aggregates the measure.
    let [metric] = ext.metrics.as_slice() else {
        panic!("one metric: {:?}", ext.metrics)
    };
    assert_eq!(metric.source_measure.as_deref(), Some(key));
    assert_eq!(metric.display_kind.as_deref(), Some("findings"));
    // A Starlark fact collector on snapshot.taken over that measure.
    let [collector] = ext.collectors.as_slice() else {
        panic!("one collector: {:?}", ext.collectors)
    };
    assert_eq!(collector.facts, vec![key.to_string()]);
    // A launcher page whose rows link to the file and line.
    let link = ext.page_files[0]
        .nodes()
        .into_iter()
        .find_map(|n| match &n.kind {
            oxplow_app::pages::schema::NodeKind::Viz { config, .. } => {
                config.columns.iter().find_map(|c| c.link.clone())
            }
            _ => None,
        })
        .expect("a linked column");
    assert_eq!(link.kind, oxplow_app::pages::spec::LinkKind::File);
    assert_eq!(link.line.as_deref(), Some("line"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_command_checks_tests_and_runs_through_the_bus() {
    let dir = project().await;
    scaffolded(dir.path(), "command", "add-note").await;
    assert!(tested(dir.path(), "add-note")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "add-note");
    let human = oxplow_domain::Actor::Human;
    let task = svc
        .commands
        .run(
            &human,
            "oxplow.work_item.create",
            serde_json::json!({ "title": "Look" }),
            true,
        )
        .await
        .unwrap();
    let item = task.result["uri"].as_str().unwrap().to_string();
    let ran = svc
        .commands
        .run(
            &human,
            &ext.commands[0].name,
            serde_json::json!({ "uri": item }),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        ran.result["children"][0]["name"], "oxplow.work_item.comment",
        "{}",
        ran.result
    );
}

/// `extension new job` checks clean, and its fixtures run the job on the
/// fixture model: the reply is the job's text, and the finished job reaches
/// `publish`, which comments it on the work item.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_job_checks_tests_and_answers_with_the_fixture_model() {
    let dir = project().await;
    scaffolded(dir.path(), "job", "note-writer").await;
    let ran = tested(dir.path(), "note-writer").await;
    for example in ["example basic", "example published"] {
        assert!(ran.contains(&example.to_string()), "{example} in {ran:?}");
    }
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "note-writer");
    for command in ["job.ask", "job.status", "job.cancel", "note.ask"] {
        let name = format!("{}.{command}", ext.namespace);
        assert!(
            ext.commands.iter().any(|c| c.name == name),
            "{name} in {:?}",
            ext.commands.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_effect_checks_tests_and_once_started_reacts() {
    use oxplow_app::effect_triggers::EffectTriggers;
    use oxplow_app::event_pump::AsyncEventConsumer as _;
    let dir = project().await;
    scaffolded(dir.path(), "effect", "done-notes").await;
    assert!(tested(dir.path(), "done-notes")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "done-notes");
    let decl = &ext.effects[0];
    oxplow_app::effects::start_at_head(&svc.db, &decl.name())
        .await
        .unwrap();
    svc.streams.ensure_primary().await.unwrap();
    let thread = oxplow_domain::ThreadId::new(1);
    let human = oxplow_domain::Actor::Human;
    let task = svc
        .commands
        .run(
            &human,
            "oxplow.work_item.create",
            serde_json::json!({ "title": "Look" }),
            true,
        )
        .await
        .unwrap();
    let item = task.result["uri"].as_str().unwrap().to_string();
    svc.commands
        .run(
            &human,
            "oxplow.work_item.transition",
            serde_json::json!({ "uri": item, "to": "done" }),
            true,
        )
        .await
        .unwrap();
    let consumer = EffectTriggers::new(std::sync::Arc::downgrade(&svc));
    for mut e in svc.event_log_store.read_after(0, 10_000).await.unwrap() {
        if consumer.handles(&e.envelope.event_type) {
            // An effect runs for its event's thread; the person's command
            // carries none, so anchor the event to the project's thread.
            e.envelope.anchors.thread_id = Some(thread);
            consumer.handle(&e).await.unwrap();
        }
    }
    let runs = svc
        .sql
        .query_sql("SELECT state FROM v_effect_run", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        runs.rows,
        vec![vec![oxplow_db::SqlCell::Text("ok".into())]],
        "the effect commented on the finished item"
    );
}

/// P9.A4: a scaffolded custom component is a private extension whose
/// bundle talks to oxplow through the served client library.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_component_checks_tests_and_uses_the_client_library() {
    let dir = project().await;
    scaffolded(dir.path(), "component", "burn-down").await;
    assert!(tested(dir.path(), "burn-down")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "burn-down");
    assert_eq!(ext.custom_components.len(), 1);
    let component = &ext.custom_components[0];
    let html = std::fs::read_to_string(
        dir.path()
            .join(&ext.path)
            .join(&component.bundle)
            .join("index.html"),
    )
    .unwrap();
    assert!(
        html.contains("/component-lib/oxplow-component.js"),
        "{html}"
    );
    assert!(
        ext.page_files
            .iter()
            .any(|p| p.nodes().iter().any(|n| matches!(
                &n.kind,
                oxplow_app::pages::schema::NodeKind::Component { .. }
            ))),
        "a page's component node draws it"
    );
}

/// The fake provider's binary, built beside this test binary.
fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let bin = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps")
        .join("oxplow-provider-fake");
    assert!(
        bin.is_file(),
        "{} is missing; build it with `cargo build -p oxplow-provider-fake`",
        bin.display()
    );
    bin
}

/// A provider script running the fake with `hooks`; its service's state
/// outlives its process, as a real service's does — what the kit's
/// restart re-sends against. The state sits outside the extension folder,
/// whose contents its consent covers.
fn fake_script(project: &Path, hooks: &str) -> String {
    format!(
        "#!/bin/sh\nOXPLOW_FAKE_HOOKS='{hooks}' OXPLOW_FAKE_STATE=\"{}/fake-state-$OXPLOW_PROVIDER_ID.json\" exec '{}' \"$@\"\n",
        project.join(".oxplow").display(),
        fake_bin().display()
    )
}

/// The kit run against a service with nothing in it: the golden
/// transcript records the refs it hands out.
async fn kit_on_a_fresh_service(
    project: &Path,
    bless: bool,
) -> oxplow_sdk::extension_test::TestReport {
    let kept = project.join(".oxplow");
    let entries = match std::fs::read_dir(&kept) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        read => read.unwrap().collect(),
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("fake-state-"))
        {
            std::fs::remove_file(path).unwrap();
        }
    }
    test_extension(project, "fake", bless).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_provider_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffolded(dir.path(), "provider", "fake").await;
    let report = test_extension(dir.path(), "fake", false).await.unwrap();
    assert!(
        report
            .error_lines()
            .join("\n")
            .contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join(".oxplow/extensions/fake");
    std::fs::write(ext.join("bin/provider"), fake_script(dir.path(), "")).unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-fake.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/basic.yaml"),
        "when: { command: create, input: { title: First } }\nexpect: { uri: $any }\n",
    )
    .unwrap();
    let blessed = kit_on_a_fresh_service(dir.path(), true).await;
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = kit_on_a_fresh_service(dir.path(), false).await;
    assert_eq!(report.error_lines(), Vec::<String>::new());
    for ran in ["work_items suite", "read work_items", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }

    // P7.A7: a cursor that doesn't advance — a second read from its last
    // checkpoint streams everything again — fails the kit.
    std::fs::write(
        ext.join("bin/provider"),
        fake_script(dir.path(), "stuck-cursor"),
    )
    .unwrap();
    let report = kit_on_a_fresh_service(dir.path(), false).await;
    assert!(
        report
            .error_lines()
            .iter()
            .any(|e| e.contains("collector `work_items`") && e.contains("doesn't advance")),
        "{:?}",
        report.errors
    );
}

/// A scaffolded effort policy is red until a program speaks for it; the
/// fake in policy mode behind it passes the kit with the scaffold's own
/// example (a checkpoint it skips).
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_effort_policy_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffold(
        dir.path(),
        Kind::provider("effort_policy").unwrap(),
        "fake",
        Some("oxplow:oxplow/effort?id=eff1"),
    )
    .unwrap();
    let report = test_extension(dir.path(), "fake", false).await.unwrap();
    assert!(
        report
            .error_lines()
            .join("\n")
            .contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join(".oxplow/extensions/fake");
    std::fs::write(
        ext.join("bin/provider"),
        fake_script(dir.path(), "").replace(
            "#!/bin/sh\n",
            "#!/bin/sh\nexport OXPLOW_FAKE_CAPABILITY=effort_policy\n",
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::policy_declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-fake.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    let blessed = kit_on_a_fresh_service(dir.path(), true).await;
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = kit_on_a_fresh_service(dir.path(), false).await;
    assert_eq!(report.error_lines(), Vec::<String>::new());
    for ran in ["provider fake", "effort_policy suite"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
    // The scaffold's example ran: one expecting commands for that
    // checkpoint fails, naming its fixture.
    let fixture = ext.join("fixtures/basic.yaml");
    let text = std::fs::read_to_string(&fixture).unwrap();
    std::fs::write(
        &fixture,
        text.replace("expect: { skip: $any }", "expect: { commands: $any }"),
    )
    .unwrap();
    let report = kit_on_a_fresh_service(dir.path(), false).await;
    assert!(
        report
            .error_lines()
            .iter()
            .any(|e| e.contains("fixtures/basic.yaml")),
        "{:?}",
        report.errors
    );
}

/// A scaffolded effort policy script checks, its example dry-runs, and it
/// passes the effort-policy suite as written.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_policy_checks_tests_and_passes_the_suite() {
    let dir = project().await;
    scaffolded(dir.path(), "policy", "tidy-up").await;
    let ran = tested(dir.path(), "tidy-up").await;
    for want in ["example basic", "effort_policy suite"] {
        assert!(ran.iter().any(|r| r == want), "{want}: {ran:?}");
    }
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "tidy-up");
    let decl = &ext.implementations[0];
    assert_eq!(
        (decl.capability.as_str(), decl.id.as_str()),
        ("effort_policy", "tidy_up")
    );
    assert_eq!(decl.needs, ["sql.read"]);
}

/// A scaffolded harness provider is red until a program speaks for it; the
/// fake in harness mode behind it passes the kit — its scaffolded example
/// (an acknowledgement rendered) and the agent-harness suite.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_harness_provider_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffold(
        dir.path(),
        Kind::provider("agent_harness").unwrap(),
        "relay",
        Some("oxplow:oxplow/effort?id=eff1"),
    )
    .unwrap();
    let env = harness_env();
    let report = test_extension_in(dir.path(), "relay", false, env.0.clone())
        .await
        .unwrap();
    assert!(
        report
            .error_lines()
            .join("\n")
            .contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join(".oxplow/extensions/relay");
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=agent_harness exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::harness_declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-relay.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    let blessed = test_extension_in(dir.path(), "relay", true, env.0.clone())
        .await
        .unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension_in(dir.path(), "relay", false, env.0)
        .await
        .unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "agent_harness suite"),
        "{:?}",
        report.ran
    );
}

/// The kit's environment for a harness provider: this process's, its PATH
/// led by a scratch dir holding a stand-in for the fake harness's binary
/// (the dir kept alive beside it).
fn harness_env() -> (oxplow_app::providers::host::HostEnv, tempfile::TempDir) {
    let bins = tempfile::tempdir().unwrap();
    std::fs::write(bins.path().join(oxplow_harness_fake::BIN), "").unwrap();
    let path = format!(
        "{}:{}",
        bins.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let env: oxplow_app::providers::host::HostEnv = std::sync::Arc::new(move |name| {
        if name == "PATH" {
            Some(path.clone())
        } else {
            std::env::var(name).ok()
        }
    });
    (env, bins)
}

/// `extension test` runs the agent-harness suite over a harness provider —
/// here the fake in harness mode — its launch looking for programs on the
/// kit environment's PATH (where a stand-in for the fake harness's binary
/// sits); `ran` shows it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_the_harness_suite_over_a_harness_provider() {
    let dir = project().await;
    let ext = dir.path().join(".oxplow/extensions/relay");
    std::fs::create_dir_all(ext.join("bin")).unwrap();
    std::fs::create_dir_all(ext.join("fixtures")).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: relay\nintent:\n  purpose: A harness over the protocol.\n  examples:\n    - { name: basic, input: { command: render, input: { answer: { kind: ack } } }, expect: its acknowledgement }\npermissions: [process.run]\nproviders:\n  - id: relay\n    capability: agent_harness\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/basic.yaml"),
        "when: { command: render, input: { answer: { kind: ack } } }\nexpect: { body: $any }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-relay.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::harness_declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=agent_harness exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            ext.join("bin/provider"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let (env, _bins) = harness_env();
    let blessed = test_extension_in(dir.path(), "relay", true, env.clone())
        .await
        .unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension_in(dir.path(), "relay", false, env)
        .await
        .unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "agent_harness suite"),
        "{:?}",
        report.ran
    );
}

/// `extension test` runs the knowledge suite over a knowledge provider —
/// here the fake in knowledge mode — made the throwaway project's knowledge
/// store; `ran` shows it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_the_knowledge_suite_over_a_knowledge_provider() {
    let dir = project().await;
    let ext = dir.path().join(".oxplow/extensions/shelf");
    std::fs::create_dir_all(ext.join("bin")).unwrap();
    std::fs::create_dir_all(ext.join("fixtures")).unwrap();
    let write = "{ command: write_page, input: { slug: example, body: '# Example', verified_files: [], removed_files: [] } }";
    std::fs::write(
        ext.join("extension.yaml"),
        format!("manifest: 2\nname: shelf\nintent:\n  purpose: Knowledge over the protocol.\n  examples:\n    - {{ name: basic, input: {write}, expect: the page }}\npermissions: [process.run]\nproviders:\n  - id: shelf\n    capability: knowledge\n    entry: bin/provider\n    declarations: provider.json\n"),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/basic.yaml"),
        format!("when: {write}\nexpect: {{ page: example }}\n"),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-shelf.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::knowledge_declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=knowledge exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            ext.join("bin/provider"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let blessed = test_extension(dir.path(), "shelf", true).await.unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension(dir.path(), "shelf", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "knowledge suite"),
        "{:?}",
        report.ran
    );
}

/// `extension test` runs the snapshot suite over a snapshots provider —
/// here the fake in snapshots mode, keeping contents — made the throwaway
/// project's snapshots; `ran` shows it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_the_snapshot_suite_over_a_snapshots_provider() {
    let dir = project().await;
    let ext = dir.path().join(".oxplow/extensions/shutter");
    let tree = dir.path().join("example-tree");
    std::fs::create_dir_all(ext.join("bin")).unwrap();
    std::fs::create_dir_all(ext.join("fixtures")).unwrap();
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("a.txt"), "alpha").unwrap();
    let mark = format!(
        "{{ command: mark, input: {{ stream: 'oxplow:oxplow/stream?id=str1', worktree: '{}', on: manual, parent: null }} }}",
        tree.display()
    );
    std::fs::write(
        ext.join("extension.yaml"),
        format!("manifest: 2\nname: shutter\nintent:\n  purpose: Snapshots over the protocol.\n  examples:\n    - {{ name: basic, input: {mark}, expect: a handle }}\npermissions: [process.run]\nproviders:\n  - id: shutter\n    capability: snapshots\n    entry: bin/provider\n    declarations: provider.json\n"),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/basic.yaml"),
        format!("when: {mark}\nexpect: {{ handle: $any, unchanged: false, file_count: 1 }}\n"),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-shutter.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::snapshots_declarations(true)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=snapshots OXPLOW_FAKE_FEATURES=contents exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            ext.join("bin/provider"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let blessed = test_extension(dir.path(), "shutter", true).await.unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension(dir.path(), "shutter", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "snapshots suite"),
        "{:?}",
        report.ran
    );
}

/// A scaffolded snapshots provider is red until a program speaks for it; the
/// fake in snapshots mode behind it passes the kit — its scaffolded example
/// (a mark of the extension's own directory) and the snapshots suite.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_snapshots_provider_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffold(
        dir.path(),
        Kind::provider("snapshots").unwrap(),
        "shutter",
        Some("oxplow:oxplow/effort?id=eff1"),
    )
    .unwrap();
    let report = test_extension(dir.path(), "shutter", false).await.unwrap();
    assert!(
        report
            .error_lines()
            .join("\n")
            .contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join(".oxplow/extensions/shutter");
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=snapshots exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::snapshots_declarations(false)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-shutter.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    let blessed = test_extension(dir.path(), "shutter", true).await.unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension(dir.path(), "shutter", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "snapshots suite"),
        "{:?}",
        report.ran
    );
}

/// A scaffolded knowledge provider is red until a program speaks for it; the
/// fake in knowledge mode behind it passes the kit — its scaffolded example
/// (a page written) and the knowledge suite.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_knowledge_provider_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffold(
        dir.path(),
        Kind::provider("knowledge").unwrap(),
        "shelf",
        Some("oxplow:oxplow/effort?id=eff1"),
    )
    .unwrap();
    let report = test_extension(dir.path(), "shelf", false).await.unwrap();
    assert!(
        report
            .error_lines()
            .join("\n")
            .contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join(".oxplow/extensions/shelf");
    std::fs::write(
        ext.join("bin/provider"),
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=knowledge exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::knowledge_declarations()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        ext.join("fixtures/provider-shelf.yaml"),
        "config: { team: core }\n",
    )
    .unwrap();
    let blessed = test_extension(dir.path(), "shelf", true).await.unwrap();
    assert_eq!(blessed.error_lines(), Vec::<String>::new());
    let report = test_extension(dir.path(), "shelf", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "knowledge suite"),
        "{:?}",
        report.ran
    );
}

/// The fields `scripts/record-just-works.sh` strips from a recorded
/// `run.json`: the author's machine (denied commands carry local paths),
/// the session and the cost.
const SCRUBBED: [&str; 5] = [
    "permission_denials",
    "session_id",
    "uuid",
    "total_cost_usd",
    "modelUsage",
];

/// P7 review (tsk723): a recorded run carries nothing of the machine or
/// session it was recorded on.
#[test]
fn recorded_runs_carry_no_local_paths_session_or_cost() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/just-works");
    for kind in std::fs::read_dir(&fixtures).unwrap() {
        let run = kind.unwrap().path().join("run.json");
        let Ok(text) = std::fs::read_to_string(&run) else {
            continue;
        };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        for field in SCRUBBED {
            assert!(json.get(field).is_none(), "{}: {field}", run.display());
        }
        for local in ["/Users/", "/home/", "/private/", "/var/folders/"] {
            assert!(!text.contains(local), "{}: {local}", run.display());
        }
    }
}

/// P7.C6: what a fresh agent built with nothing but the skill (recorded
/// once by `scripts/record-just-works.sh <kind>`, see its notes.md) still
/// checks and tests clean against this oxplow.
#[tokio::test(flavor = "multi_thread")]
async fn recorded_agent_runs_still_check_and_test_clean() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/just-works");
    let mut replayed = 0;
    for kind in std::fs::read_dir(&fixtures).unwrap() {
        let kind = kind.unwrap().path();
        if !kind.join("prompt.md").is_file() {
            continue;
        }
        let produced = kind.join("produced");
        assert!(
            produced.is_dir(),
            "{} has a prompt but no recorded run — record it with \
             `scripts/record-just-works.sh {}`",
            kind.display(),
            kind.file_name().unwrap().to_string_lossy()
        );
        let dir = project().await;
        let extensions = dir.path().join(".oxplow/extensions");
        std::fs::create_dir_all(&extensions).unwrap();
        for ext in std::fs::read_dir(&produced).unwrap() {
            let ext = ext.unwrap().path();
            let name = ext.file_name().unwrap().to_string_lossy().to_string();
            copy(&ext, &extensions.join(&name));
            let report = check(
                dir.path(),
                &name,
                &ExtensionCatalog::new(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
            assert!(report.ok, "{}", render_findings(&report, Format::Text));
            let tested = test_extension(dir.path(), &name, false).await.unwrap();
            assert_eq!(tested.error_lines(), Vec::<String>::new(), "{name}");
            replayed += 1;
        }
    }
    assert!(
        replayed > 0,
        "no recorded runs under {}",
        fixtures.display()
    );
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_advisory_checks_loads_and_stays_quiet() {
    let dir = project().await;
    scaffolded(dir.path(), "advisory", "watch-effort").await;
    // Like a bare extension: its example has nothing to run (`extension
    // test` runs pages, collectors, commands, effects and providers), so
    // that warning is the only one.
    let report = test_extension(dir.path(), "watch-effort", false)
        .await
        .unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "watch-effort");
    assert_eq!(ext.advisories.len(), 1);
    assert_eq!(ext.advisories[0].id, "watch-effort");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_skill_checks_and_is_declared() {
    let dir = project().await;
    scaffolded(dir.path(), "skill", "house-style").await;
    let report = test_extension(dir.path(), "house-style", false)
        .await
        .unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "house-style");
    assert_eq!(ext.skills.len(), 1);
    assert_eq!(ext.skills[0].name, "house-style");
    assert_eq!(ext.skills[0].file, "skills/house-style/SKILL.md");
}

/// A scaffolded query checks clean, its fixture runs it as a query, and it
/// loads as one: rows only, no view of its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_query_checks_tests_and_loads_as_a_query() {
    let dir = project().await;
    scaffolded(dir.path(), "query", "item-states").await;
    assert!(tested(dir.path(), "item-states")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "item-states");
    assert_eq!(ext.page_files.len(), 1);
    assert_eq!(
        ext.page_files[0].kind,
        oxplow_app::pages::schema::PageKind::Query
    );
    assert_eq!(ext.page_files[0].file(), "queries/item-states.yaml");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_panel_checks_tests_and_shows_its_page() {
    let dir = project().await;
    scaffolded(dir.path(), "panel", "open-work").await;
    assert!(tested(dir.path(), "open-work")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "open-work");
    assert_eq!(ext.panels.len(), 1);
    assert_eq!(ext.panels[0].body, "open_work/open-work");
    assert_eq!(ext.page_files.len(), 1);
}

/// The shipped extensions pass `extension test`, their pages' answers run.
#[tokio::test(flavor = "multi_thread")]
async fn the_shipped_extensions_pass_their_own_test() {
    let dir = project().await;
    for name in ["oxplow-foundation", "oxplow-bundled"] {
        let report = test_extension(dir.path(), name, false).await.unwrap();
        assert_eq!(report.error_lines(), Vec::<String>::new(), "{name}");
        if name == "oxplow-foundation" {
            assert!(
                report.ran.contains(&"answers".to_string()),
                "{:?}",
                report.ran
            );
        }
    }
}
