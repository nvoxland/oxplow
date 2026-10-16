//! The triage example's workflow state — each TODO's decision, kept in the
//! extension's private state — survives what a person does around it: a
//! restart, revoking the board's approval, an edit to the extension that
//! asks for the approval again. Over an on-disk database, so a restart is
//! real.

use std::path::Path;
use std::sync::Arc;

use oxplow_app::exec_consent::{approve_every, approve_program, revoke_program, ProgramKind};
use oxplow_app::lens_actions::{invoke_component_command, load_component, ComponentInvoke};
use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

const LENS: &str = "triage/todo-board";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("it works");
    for entry in std::fs::read_dir(from).expect("it works") {
        let entry = entry.expect("it works");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("it works").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("it works");
        }
    }
}

struct Project {
    dir: tempfile::TempDir,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
}

impl Project {
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("it works");
        oxplow_app::vcs::GitProvider
            .init_repository(dir.path())
            .await
            .expect("it works");
        let example =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/triage");
        copy_dir(&example, &dir.path().join("oxplow/extensions/triage"));
        Self {
            dir,
            secrets: Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        }
    }

    /// A running oxplow over the project's own database file, started as
    /// the daemon starts it: opening it again is a restart. Its approvals
    /// live in the machine's config dir, here the project's.
    async fn open(&self) -> Arc<oxplow_app::Services> {
        std::env::set_var("OXPLOW_HOME", self.dir.path().join(".oxplow/global-config"));
        let layout = oxplow_app::AppLayout::for_project(self.dir.path());
        let svc =
            Arc::new(oxplow_app::Services::boot(layout, self.secrets.clone()).expect("it works"));
        svc.extension_activation
            .activate(svc.clone())
            .await
            .expect("it works");
        svc
    }
}

/// `command` as the todo-board's frame would run it, at the version it was
/// loaded at.
async fn as_board(
    svc: &oxplow_app::Services,
    root: &Path,
    command: &str,
    input: Value,
) -> Result<Value, CommandError> {
    let version = load_component(svc, root, LENS).expect("it works");
    invoke_component_command(
        svc,
        root,
        ComponentInvoke {
            lens_id: LENS.into(),
            command: command.into(),
            input,
            on_behalf_of: Actor::Human,
            confirmed: true,
            version,
        },
    )
    .await
    .map(|out| out.result)
}

fn decide_input(line: i64, decision: &str) -> Value {
    json!({ "path": "src/lib.rs", "line": line, "decision": decision })
}

/// The decisions the board reads back.
async fn decisions(svc: &oxplow_app::Services, root: &Path) -> Value {
    let out = as_board(svc, root, "triage.todos.decisions", json!({}))
        .await
        .expect("it works");
    out.get("result").unwrap_or(&out)["decisions"].clone()
}

fn component_version(svc: &oxplow_app::Services, root: &Path) -> String {
    load_component(svc, root, LENS).expect("it works")
}

fn approve_board(svc: &oxplow_app::Services, root: &Path, version: &str) -> Result<(), String> {
    let extensions = svc.extension_catalog.get(root);
    approve_program(
        &svc.approvals,
        root,
        &extensions,
        ProgramKind::Component,
        LENS,
        version,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decision_survives_a_restart() {
    let project = Project::new().await;
    let root = project.dir.path();
    let before = project.open().await;
    approve_every(&before, ProgramKind::Component).expect("it works");
    as_board(
        &before,
        root,
        "triage.todos.decide",
        decide_input(1, "fix-now"),
    )
    .await
    .expect("it works");
    assert_eq!(
        decisions(&before, root).await,
        json!({ "src/lib.rs:1": "fix-now" })
    );
    drop(before);

    let after = project.open().await;
    assert_eq!(
        decisions(&after, root).await,
        json!({ "src/lib.rs:1": "fix-now" }),
        "the approval and the decision are both still there"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_board_stops_it_deciding_and_approving_again_keeps_what_it_decided() {
    let project = Project::new().await;
    let root = project.dir.path();
    let svc = project.open().await;
    approve_every(&svc, ProgramKind::Component).expect("it works");
    as_board(&svc, root, "triage.todos.decide", decide_input(1, "keep"))
        .await
        .expect("it works");

    let extensions = svc.extension_catalog.get(root);
    revoke_program(
        &svc.approvals,
        root,
        &extensions,
        ProgramKind::Component,
        LENS,
    )
    .expect("it works");
    let refused = as_board(
        &svc,
        root,
        "triage.todos.decide",
        decide_input(2, "wontfix"),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&refused, CommandError::Denied { reason } if reason.contains("Settings → Data → Programs")),
        "{refused:?}"
    );

    approve_every(&svc, ProgramKind::Component).expect("it works");
    as_board(
        &svc,
        root,
        "triage.todos.decide",
        decide_input(2, "wontfix"),
    )
    .await
    .expect("it works");
    assert_eq!(
        decisions(&svc, root).await,
        json!({ "src/lib.rs:1": "keep", "src/lib.rs:2": "wontfix" }),
        "the earlier decision is still there"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_bundle_asks_for_the_approval_again_and_the_decisions_read_back() {
    let project = Project::new().await;
    let root = project.dir.path();
    let svc = project.open().await;
    approve_every(&svc, ProgramKind::Component).expect("it works");
    as_board(&svc, root, "triage.todos.decide", decide_input(1, "keep"))
        .await
        .expect("it works");
    let reviewed = component_version(&svc, root);

    // A comment in the manifest isn't what they reviewed: the bundle and
    // the commands it may run are, so the approval stands.
    let manifest = root.join("oxplow/extensions/triage/extension.yaml");
    let text = std::fs::read_to_string(&manifest).expect("it works");
    std::fs::write(&manifest, format!("# reviewed again\n{text}")).expect("it works");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("it works");
    assert_eq!(component_version(&svc, root), reviewed);

    // The bundle changes after they reviewed it.
    let script = root.join("oxplow/extensions/triage/components/todo-board/app.js");
    let code = std::fs::read_to_string(&script).expect("it works");
    std::fs::write(&script, format!("{code}\n// edited\n")).expect("it works");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("it works");

    let stale = approve_board(&svc, root, &reviewed).unwrap_err();
    assert!(stale.contains("changed since you reviewed it"), "{stale}");
    let refused = as_board(&svc, root, "triage.todos.decisions", json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, CommandError::Denied { .. }),
        "approval is asked for again: {refused:?}"
    );

    approve_board(&svc, root, &component_version(&svc, root)).expect("it works");
    assert_eq!(
        decisions(&svc, root).await,
        json!({ "src/lib.rs:1": "keep" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_stamp_is_refused_and_each_decision_reads_its_own() {
    let project = Project::new().await;
    let root = project.dir.path();
    let svc = project.open().await;
    approve_every(&svc, ProgramKind::Component).expect("it works");
    as_board(&svc, root, "triage.todos.decide", decide_input(1, "keep"))
        .await
        .expect("it works");

    // A write that still thinks nothing was ever decided.
    let stale = svc
        .commands
        .run(
            &Actor::Lens {
                lens_id: LENS.into(),
                on_behalf_of: Box::new(Actor::Human),
            },
            "triage.state.set",
            json!({
                "scope": "project", "area": "state", "expected": null,
                "schema_version": 1, "values": { "decisions": { "src/lib.rs:9": "keep" } },
            }),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        stale.to_string().to_lowercase().contains("stamp")
            || stale.to_string().to_lowercase().contains("conflict")
            || stale.to_string().to_lowercase().contains("changed"),
        "a diagnostic naming the conflict: {stale}"
    );

    // Two decisions in a row each read the stamp the last one left.
    as_board(
        &svc,
        root,
        "triage.todos.decide",
        decide_input(2, "fix-now"),
    )
    .await
    .expect("it works");
    as_board(
        &svc,
        root,
        "triage.todos.decide",
        decide_input(3, "wontfix"),
    )
    .await
    .expect("it works");
    assert_eq!(
        decisions(&svc, root).await,
        json!({ "src/lib.rs:1": "keep", "src/lib.rs:2": "fix-now", "src/lib.rs:3": "wontfix" })
    );
}
