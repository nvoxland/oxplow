//! P7.C6: "just works", deterministically. For each kind `oxplow plugin
//! new` makes: a temp project → scaffold → `check` (clean, no warnings) →
//! `plugin test` (clean) → it loads in a real oxplow and does its job —
//! a lens's row action binds, a collector's entity and model publish, a
//! command runs through the bus. A provider's stub is red until a real
//! program stands behind it (the fake).

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_sdk::{check, plugin_test::test_extension, render_findings, scaffold, Format, Kind};

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
    let kind = Kind::parse(kind).unwrap_or_else(|| panic!("`plugin new {kind}`"));
    scaffold(root, kind, name, Some("effort:eff1")).unwrap();
    let report = check(root, name, &ExtensionCatalog::new(), None, None, None)
        .await
        .unwrap();
    assert!(report.ok, "{}", render_findings(&report, Format::Text));
    assert_eq!(report.warnings, Vec::<String>::new(), "{name}");
}

/// `plugin test` is clean; what ran.
async fn tested(root: &Path, name: &str) -> Vec<String> {
    let report = test_extension(root, name, false).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new(), "{name}");
    assert_eq!(report.warnings, Vec::<String>::new(), "{name}");
    report.ran
}

/// A real oxplow over the project, as at boot.
async fn booted(root: &Path) -> oxplow_app::Services {
    let svc = oxplow_app::Services::in_memory(root).unwrap();
    svc.extension_models.sync().await.unwrap();
    svc.extension_commands.reconcile().await;
    svc
}

fn loaded(
    svc: &oxplow_app::Services,
    root: &Path,
    name: &str,
) -> oxplow_app::extensions::Extension {
    let ext = svc
        .extension_catalog
        .get(root)
        .iter()
        .find(|e| e.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} loads"));
    assert_eq!(ext.errors, Vec::<String>::new(), "{name}");
    ext
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_lens_checks_tests_and_its_row_action_binds() {
    let dir = project().await;
    scaffolded(dir.path(), "lens", "open-work").await;
    assert!(tested(dir.path(), "open-work")
        .await
        .contains(&"example basic".to_string()));
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "open-work");
    let lens = &ext.lenses[0];
    let action = lens.actions.iter().find(|a| a.row).expect("a row action");
    assert!(
        svc.commands.spec(&action.command).is_some(),
        "{}",
        action.command
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_extension_checks_and_loads() {
    let dir = project().await;
    scaffolded(dir.path(), "extension", "my-notes").await;
    let report = test_extension(dir.path(), "my-notes", false).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
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
    let svc = booted(dir.path()).await;
    let ext = loaded(&svc, dir.path(), "open-items");
    let collector = &ext.collectors[0];
    // It runs for real: its entity, then its model, publish.
    svc.commands
        .run(
            &oxplow_domain::Actor::Human,
            "collector.sync",
            serde_json::json!({ "owner": "open-items", "id": collector.id }),
            true,
        )
        .await
        .unwrap();
    svc.extension_models.sync().await.unwrap();
    let lens = &ext.lenses[0];
    let run = oxplow_app::extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        dir.path(),
        &lens.id,
        Default::default(),
        &oxplow_app::extensions::LensContext::default(),
    )
    .await
    .unwrap();
    assert!(
        run.result.columns.iter().any(|c| c == "title"),
        "{:?}",
        run.result.columns
    );
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
            "work_item.create",
            serde_json::json!({ "title": "Look" }),
            true,
        )
        .await
        .unwrap();
    let item = format!("work_item:oxplow:{}", task.result["id"].as_str().unwrap());
    let ran = svc
        .commands
        .run(
            &human,
            &ext.commands[0].name,
            serde_json::json!({ "ref": item }),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        ran.result["children"][0]["name"], "work_item.comment",
        "{}",
        ran.result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_effect_checks_tests_and_once_approved_reacts() {
    use oxplow_app::effect_triggers::EffectTriggers;
    use oxplow_app::event_pump::AsyncEventConsumer as _;
    use oxplow_app::exec_consent::{approve_program, ProgramKind};
    let dir = project().await;
    scaffolded(dir.path(), "effect", "done-notes").await;
    assert!(tested(dir.path(), "done-notes")
        .await
        .contains(&"example basic".to_string()));
    let svc = std::sync::Arc::new(booted(dir.path()).await);
    let ext = loaded(&svc, dir.path(), "done-notes");
    let decl = &ext.effects[0];
    let program = oxplow_app::effects::effect_program(&ext, decl);
    let config = svc.config.read().unwrap().clone();
    approve_program(
        &svc.approvals,
        dir.path(),
        &config,
        std::slice::from_ref(&ext),
        ProgramKind::Effect,
        &program.name,
        &program.hash(dir.path()).unwrap(),
    )
    .unwrap();
    oxplow_app::effects::approved(&svc.db, &decl.name())
        .await
        .unwrap();
    let human = oxplow_domain::Actor::Human;
    let task = svc
        .commands
        .run(
            &human,
            "work_item.create",
            serde_json::json!({ "title": "Look" }),
            true,
        )
        .await
        .unwrap();
    let item = format!("work_item:oxplow:{}", task.result["id"].as_str().unwrap());
    svc.commands
        .run(
            &human,
            "work_item.transition",
            serde_json::json!({ "ref": item, "to": "done" }),
            true,
        )
        .await
        .unwrap();
    let consumer = EffectTriggers::new(std::sync::Arc::downgrade(&svc));
    for e in svc.event_log_store.read_after(0, 10_000).await.unwrap() {
        if consumer.handles(&e.envelope.event_type) {
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

/// The fake provider's binary, built beside this test binary.
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
    assert_eq!(ext.sharing, oxplow_app::extensions::Sharing::Private);
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
        ext.lenses.iter().any(|l| l.custom.is_some()),
        "a `viz: custom` lens renders it"
    );
}

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

#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_provider_is_red_until_a_program_speaks_for_it() {
    let dir = project().await;
    scaffolded(dir.path(), "provider", "fake").await;
    let report = test_extension(dir.path(), "fake", false).await.unwrap();
    assert!(
        report.errors.join("\n").contains("initialize failed"),
        "{:?}",
        report.errors
    );
    let ext = dir.path().join("oxplow/extensions/fake");
    std::fs::write(
        ext.join("bin/provider"),
        format!("#!/bin/sh\nexec '{}' \"$@\"\n", fake_bin().display()),
    )
    .unwrap();
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
        "input: { command: create, input: { title: First } }\nexpect: { ref: $any }\n",
    )
    .unwrap();
    let blessed = test_extension(dir.path(), "fake", true).await.unwrap();
    assert_eq!(blessed.errors, Vec::<String>::new());
    let report = test_extension(dir.path(), "fake", false).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
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
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_HOOKS=stuck-cursor exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    let report = test_extension(dir.path(), "fake", false).await.unwrap();
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("collector `work_items`") && e.contains("doesn't advance")),
        "{:?}",
        report.errors
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
        let extensions = dir.path().join("oxplow/extensions");
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
            assert_eq!(tested.errors, Vec::<String>::new(), "{name}");
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
