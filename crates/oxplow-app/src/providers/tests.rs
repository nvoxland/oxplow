//! P5.D3: the host over the real fake provider process — consent before
//! execution, the handshake against the approved declarations, and the
//! work-items conformance suite through `ExternalWorkItems`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// A private extension declaring the fake as its work-items provider.
fn write_extension(project: &Path) {
    let dir = project.join("oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: tracker\nsharing: private\nintent:\n  purpose: the fake tracker\n  examples: [{ name: a }]\nproviders:\n  - id: fake\n    capability: work_items\n    entry: bin/provider\n    env: [OXPLOW_FAKE_BIN, OXPLOW_FAKE_HOOKS]\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    std::fs::write(&script, "#!/bin/sh\nexec \"$OXPLOW_FAKE_BIN\" \"$@\"\n").unwrap();
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

/// A registry over the fixture's bus whose providers see `hooks`.
fn registry(fx: &EffortFixture, hooks: &str) -> ProviderRegistry {
    let bin = fake_bin().to_string_lossy().into_owned();
    let hooks = hooks.to_string();
    ProviderRegistry::new(
        HostDeps {
            project_dir: fx.svc.layout.project_dir.clone(),
            project: crate::source_runner::project_key(&fx.svc.layout.project_dir),
            approvals: fx.svc.approvals.clone(),
            secrets: fx.svc.secrets.clone(),
            host_env: Arc::new(move |name| match name {
                "OXPLOW_FAKE_BIN" => Some(bin.clone()),
                "OXPLOW_FAKE_HOOKS" => Some(hooks.clone()),
                other => std::env::var(other).ok(),
            }),
        },
        &fx.svc.commands,
        fx.svc.work_items.clone(),
    )
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

#[tokio::test]
async fn an_unapproved_provider_is_refused_and_registers_nothing() {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir);
    let ext = extension(&fx.svc.layout.project_dir);
    let listed = program(&fx, &ext);
    assert_eq!(listed.name, "tracker/fake");
    assert!(!listed.approved);
    assert_eq!(listed.env, vec!["OXPLOW_FAKE_BIN", "OXPLOW_FAKE_HOOKS"]);

    let providers = registry(&fx, "");
    let refused = providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert_eq!(refused, HostError::Unapproved("tracker/fake".into()));
    assert!(!fx.svc.commands.has_namespace("fake"));
    assert!(fx.svc.work_items.get("fake").is_err());
}

#[tokio::test]
async fn edited_declarations_need_approving_again() {
    let fx = services_with_effort().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project);
    let ext = extension(&project);
    approve(&fx, &ext);
    let providers = registry(&fx, "");
    providers
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap();
    assert!(fx.svc.commands.has_namespace("fake"));
    assert!(providers.disable("fake").await);
    assert!(!fx.svc.commands.has_namespace("fake"));

    // A widened declaration is a new version: shown unapproved, refused.
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
        HostError::Unapproved("tracker/fake".into())
    );
}

#[tokio::test]
async fn a_provider_must_answer_with_its_approved_declarations_and_a_valid_config() {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir);
    let ext = extension(&fx.svc.layout.project_dir);
    approve(&fx, &ext);

    let lying = registry(&fx, "bad-declarations");
    let err = lying
        .enable(&ext, &ext.providers[0], json!({ "team": "core" }))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, HostError::DeclarationsChanged { detail, .. } if detail.contains("/commands")),
        "{err}"
    );
    assert!(!fx.svc.commands.has_namespace("fake"));

    let unconfigured = registry(&fx, "");
    let err = unconfigured
        .enable(&ext, &ext.providers[0], json!({}))
        .await
        .unwrap_err();
    assert_eq!(err.problems()[0].path, "/team", "{err}");
}

#[tokio::test]
async fn the_work_items_suite_passes_through_the_host_over_the_fake() {
    let fx = services_with_effort().await;
    write_extension(&fx.svc.layout.project_dir);
    let ext = extension(&fx.svc.layout.project_dir);
    approve(&fx, &ext);
    let providers = registry(&fx, "");
    providers
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

    // Its commands are on the bus, External, and audited as run.
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
}
