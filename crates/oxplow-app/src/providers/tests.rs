//! P5.D3: the host over the real fake provider process — consent before
//! execution, the handshake against the approved declarations, and the
//! work-items conformance suite through `ExternalWorkItems`.

use std::path::{Path, PathBuf};

use oxplow_domain::{Actor, ThreadId};
use serde_json::json;

use super::*;
use crate::exec_consent::{self, ProgramKind};
use crate::extensions::Extension;
use crate::test_fixtures::{services_with_effort, EffortFixture};
use crate::work_items_conformance::{suite, ServicesProbe};

const EXT: &str = "tracker";

/// The fake provider's binary, built beside this test binary (the
/// workspace build builds every crate's bins).
fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps");
    let bin = dir.join("oxplow-provider-fake");
    assert!(
        bin.is_file(),
        "{} is missing; build it with `cargo build -p oxplow-provider-fake`",
        bin.display()
    );
    bin
}

/// A private extension declaring the fake as its work-items provider,
/// its entry a script running the fake with `hooks`.
fn write_extension(project: &Path, hooks: &str) {
    let dir = project.join("oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: tracker\nsharing: private\nintent:\n  purpose: the fake tracker\n  examples: [{ name: a }]\nproviders:\n  - id: fake\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_HOOKS='{hooks}' exec '{}' \"$@\"\n",
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
    )
    .unwrap();
}

fn extension(project: &Path) -> Extension {
    let ext = crate::extensions::load_extensions(project)
        .into_iter()
        .find(|e| e.name == EXT)
        .expect("the tracker extension loads");
    assert_eq!(ext.errors, Vec::<String>::new());
    ext
}

/// The listing's view of the provider, as a person sees it.
fn program(fx: &EffortFixture, ext: &Extension) -> exec_consent::ProjectProgram {
    let config = fx.svc.config.read().unwrap().clone();
    exec_consent::list(
        &fx.svc.approvals,
        &fx.svc.layout.project_dir,
        &config,
        std::slice::from_ref(ext),
    )
    .into_iter()
    .find(|p| p.kind == ProgramKind::Provider)
    .expect("the provider is listed")
}

fn approve(fx: &EffortFixture, ext: &Extension) {
    let p = program(fx, ext);
    let config = fx.svc.config.read().unwrap().clone();
    exec_consent::approve_program(
        &fx.svc.approvals,
        &fx.svc.layout.project_dir,
        &config,
        std::slice::from_ref(ext),
        ProgramKind::Provider,
        &p.name,
        p.version.as_deref().unwrap(),
    )
    .unwrap();
}

/// The fixture with the tracker extension (running the fake with
/// `hooks`) approved on this machine.
async fn approved(hooks: &str) -> (EffortFixture, Extension) {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir, hooks);
    let ext = extension(&fx.svc.layout.project_dir);
    approve(&fx, &ext);
    (fx, ext)
}

const INSTANCE: &str = "tracker/fake";

/// Enable the instance in the project's config (in memory, as a
/// reconcile reads it).
fn configure(fx: &EffortFixture, enabled: bool, config: serde_json::Value) {
    fx.svc.config.write().unwrap().extension_instances.insert(
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig { enabled, config },
    );
}

async fn logged(fx: &EffortFixture, event_type: &str) -> Vec<serde_json::Value> {
    let t = event_type.to_string();
    fx.svc
        .db
        .read(move |c| {
            let mut stmt = c
                .prepare("SELECT payload FROM event_log WHERE type = ?1 ORDER BY seq")
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map([t], |r| r.get::<_, String>(0))
                .and_then(|r| r.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            Ok(rows)
        })
        .await
        .unwrap()
        .into_iter()
        .map(|p| serde_json::from_str(&p).unwrap())
        .collect()
}

#[tokio::test]
async fn an_unapproved_provider_is_refused_and_registers_nothing() {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir, "");
    let ext = extension(&fx.svc.layout.project_dir);
    let listed = program(&fx, &ext);
    assert_eq!(listed.name, INSTANCE);
    assert!(!listed.approved);

    let refused = fx
        .svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert_eq!(refused, HostError::Unapproved(INSTANCE.into()));
    assert!(!fx.svc.commands.has_namespace("fake"));
    assert!(fx.svc.work_items.get("fake").is_err());
    assert_eq!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Unapproved
    );
}

#[tokio::test]
async fn edited_declarations_need_approving_again() {
    let (fx, ext) = approved("").await;
    let providers = &fx.svc.providers;
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    assert!(fx.svc.commands.has_namespace("fake"));
    assert!(providers.stop(INSTANCE).await);
    assert!(!fx.svc.commands.has_namespace("fake"));

    // A widened declaration is a new version: shown unapproved, refused.
    let project = fx.svc.layout.project_dir.clone();
    let file = project.join("oxplow/extensions/tracker/provider.json");
    let mut declared = oxplow_provider_fake::declarations();
    declared.commands[0].summary = "Create a work item, now everywhere.".into();
    std::fs::write(&file, serde_json::to_string(&declared).unwrap()).unwrap();
    let ext = extension(&project);
    assert!(!program(&fx, &ext).approved);
    assert_eq!(
        providers
            .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
            .await
            .unwrap_err(),
        HostError::Unapproved(INSTANCE.into())
    );
}

#[tokio::test]
async fn a_provider_must_answer_with_its_approved_declarations() {
    let (fx, ext) = approved("bad-declarations").await;
    let err = fx
        .svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, HostError::DeclarationsChanged { detail, .. } if detail.contains("/commands")),
        "{err}"
    );
    assert!(!fx.svc.commands.has_namespace("fake"));
    // Not what was approved: off, and logged, until a person looks.
    assert!(matches!(
        fx.svc.providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));
    assert_eq!(logged(&fx, "provider.disabled").await.len(), 1);
}

/// P5.D4's red: an unconfigured instance can't be enabled — the check
/// refuses it naming the config field, and nothing is written.
#[tokio::test]
async fn an_unconfigured_instance_cannot_be_enabled() {
    let (fx, _ext) = approved("").await;
    let providers = &fx.svc.providers;
    let checked = providers.check_instance(INSTANCE, json!({})).await.unwrap();
    assert!(
        matches!(&checked.health.state, InstanceState::Unconfigured { problems } if problems[0].path == "/team"),
        "{checked:?}"
    );
    let refused = providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, oxplow_domain::CommandError::Invalid { field: Some(f), .. } if f == "/config/team"),
        "{refused:?}"
    );
    assert!(fx.svc.config.read().unwrap().extension_instances.is_empty());
    assert!(!fx.svc.commands.has_namespace("fake"));

    // Configured, it enables: written to the project's config and running.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, true, json!({ "team": "core" }))
        .await
        .unwrap();
    assert_eq!(view.health.state, InstanceState::Ready);
    assert!(view.enabled);
    assert!(fx.svc.commands.has_namespace("fake"));
    let written =
        std::fs::read_to_string(oxplow_config::config_path(&fx.svc.layout.project_dir)).unwrap();
    assert!(written.contains("extensionInstances"), "{written}");
    assert_eq!(logged(&fx, "provider.enabled").await.len(), 1);

    // Disabling stops it.
    let view = providers
        .set_instance(&Actor::Human, INSTANCE, false, json!({ "team": "core" }))
        .await
        .unwrap();
    assert_eq!(view.health.state, InstanceState::Off);
    assert!(!fx.svc.commands.has_namespace("fake"));
}

/// P5.D4's red: three failures in a row disable the instance, logged
/// with the reason; it stays off across reconciles until a person runs
/// `provider.enable`.
#[tokio::test]
async fn three_failures_in_a_row_disable_an_instance_until_a_person_enables_it() {
    let (fx, _ext) = approved("fail-next:3").await;
    let providers = &fx.svc.providers;
    configure(&fx, true, json!({ "team": "core" }));

    // Every start's check fails (each process begins with fail-next:3):
    // the reconcile's start is the first failure, two calls the rest.
    providers.reconcile().await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Failing { .. }
    ));
    for _ in 0..2 {
        let failed = fx
            .svc
            .commands
            .run(&Actor::Human, "fake.create", json!({ "title": "x" }), false)
            .await;
        assert!(failed.is_err());
    }
    let health = providers.health(INSTANCE).unwrap();
    let InstanceState::Disabled { reason } = &health.state else {
        panic!("{health:?}");
    };
    assert!(reason.contains("3 failures in a row"), "{reason}");
    assert!(!fx.svc.commands.has_namespace("fake"));
    let disabled = logged(&fx, "provider.disabled").await;
    assert_eq!(disabled.len(), 1);
    assert_eq!(disabled[0]["instance"], INSTANCE);
    assert_eq!(disabled[0]["reason"], reason.as_str());

    // The log keeps it off.
    providers.reconcile().await;
    assert!(matches!(
        providers.health(INSTANCE).unwrap().state,
        InstanceState::Disabled { .. }
    ));

    // Only a person enables it again.
    let denied = fx
        .svc
        .commands
        .run(
            &Actor::Agent {
                thread_id: Some(ThreadId::new(fx.thread.value())),
                stream_id: None,
            },
            "provider.enable",
            json!({ "instance": INSTANCE }),
            false,
        )
        .await;
    assert!(matches!(
        denied,
        Err(oxplow_domain::CommandError::Denied { .. })
    ));
    write_extension(&fx.svc.layout.project_dir, "");
    approve(&fx, &extension(&fx.svc.layout.project_dir));
    let enabled = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            "provider.enable",
            json!({ "instance": INSTANCE }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        enabled.result["health"]["state"]["state"], "ready",
        "{}",
        enabled.result
    );
    assert!(fx.svc.commands.has_namespace("fake"));
}

#[tokio::test]
async fn the_work_items_suite_passes_through_the_host_over_the_fake() {
    let (fx, ext) = approved("").await;
    fx.svc
        .providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    let provider = fx.svc.work_items.get("fake").unwrap();
    let actor = Actor::Agent {
        thread_id: Some(ThreadId::new(fx.thread.value())),
        stream_id: None,
    };
    let findings = suite(&*provider, &ServicesProbe(&fx.svc), &actor).await;
    assert_eq!(findings, vec![]);

    // Its commands are on the bus, External, and audited as run; its
    // health counts the calls.
    let spec = fx.svc.commands.spec("fake.create").unwrap();
    assert_eq!(spec.atomicity, oxplow_domain::Atomicity::External);
    let audited = fx
        .svc
        .db
        .read(|c| {
            c.query_row(
                "SELECT count(*) FROM command_audit WHERE command = 'fake.transition' AND outcome = 'ok'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .await
        .unwrap();
    assert!(audited >= 5, "{audited}");
    let health = fx.svc.providers.health(INSTANCE).unwrap();
    assert_eq!(health.state, InstanceState::Ready);
    assert!(health.mean_invoke_ms.is_some() && health.last_ok_at.is_some());
}

/// tsk546: a provider's args are hashed where it runs (its extension
/// folder), and an arg reaching outside the folder is refused, so no file
/// it runs escapes the approval.
#[tokio::test]
async fn provider_args_stay_inside_the_approved_folder() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let manifest = project.join("oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    args: [../../../tools/tracker.py]\n",
        ),
    )
    .unwrap();
    let loaded = crate::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == EXT)
        .unwrap();
    assert!(loaded.providers.is_empty());
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| e.contains("../../../tools/tracker.py")),
        "{:?}",
        loaded.errors
    );

    // An arg inside the folder is part of the approval.
    std::fs::write(
        &manifest,
        text.replace(
            "    entry: bin/provider\n",
            "    entry: bin/provider\n    args: [lib/main.py]\n",
        ),
    )
    .unwrap();
    let dir = project.join("oxplow/extensions/tracker");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/main.py"), "good()").unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    assert!(program(&fx, &ext).approved);
    std::fs::write(dir.join("lib/main.py"), "evil()").unwrap();
    assert!(!program(&fx, &ext).approved);
}

/// tsk547: a provider runs from a verified copy of its approved folder,
/// outside the repo — not from the live tree — and a tampered copy is
/// replaced before it runs.
#[tokio::test]
async fn a_provider_runs_from_its_verified_copy() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let script = project.join("oxplow/extensions/tracker/bin/provider");
    let seen = tempfile::tempdir().unwrap();
    let cwd_file = seen.path().join("cwd");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\npwd > '{}'\nexec '{}' \"$@\"\n",
            cwd_file.display(),
            fake_bin().display()
        ),
    )
    .unwrap();
    let ext = extension(&project);
    approve(&fx, &ext);
    let providers = &fx.svc.providers;
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    let copies = fx
        .svc
        .layout
        .state_dir
        .join("global-config/provider-copies");
    let ran_in = std::path::PathBuf::from(std::fs::read_to_string(&cwd_file).unwrap().trim());
    let copies = copies.canonicalize().unwrap();
    assert!(
        ran_in.starts_with(&copies),
        "{} not under {}",
        ran_in.display(),
        copies.display()
    );
    assert!(
        ran_in.ends_with("oxplow/extensions/tracker"),
        "{}",
        ran_in.display()
    );

    // Someone edits the copy: the next start replaces it from the
    // approved tree before running.
    assert!(providers.stop(INSTANCE).await);
    let copied = ran_in.join("bin/provider");
    std::fs::write(&copied, "#!/bin/sh\necho tampered\nexit 1\n").unwrap();
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&copied).unwrap(),
        std::fs::read_to_string(&script).unwrap()
    );
}
