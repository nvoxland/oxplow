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
    let report = check(root, name, &ExtensionCatalog::new(), None, None)
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
    assert!(
        report.ran.iter().any(|r| r == "work_items suite"),
        "{:?}",
        report.ran
    );
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
            let report = check(dir.path(), &name, &ExtensionCatalog::new(), None, None)
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
