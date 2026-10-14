//! A collector's run holds no catalog generation while it goes on: an
//! activation never waits for it, and a change to (or the revocation of)
//! the collector's own extension stops it publishing under the old
//! declaration.

use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

use oxplow_domain::Actor;
use serde_json::json;

use crate::test_fixtures::{services_with_task_effort, TaskEffortFixture};

fn manifest(version: &str) -> String {
    format!(
        "manifest: 2\nname: acme\nversion: {version}\nintent:\n  purpose: p\ncollectors:
  - id: sync
    runtime: exec
    entry: sync.sh
    entities:
      - {{ name: seen, key: k, columns: {{ k: text, v: text }} }}
"
    )
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A program that says it started, waits to be released, then reports a row.
async fn long_collector(fx: &TaskEffortFixture) {
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        "oxplow/extensions/acme/extension.yaml",
        &manifest("1.0.0"),
    );
    let started = root.join("started");
    let release = root.join("release");
    let sync = root.join("oxplow/extensions/acme/sync.sh");
    write(
        &root,
        "oxplow/extensions/acme/sync.sh",
        &format!(
            "#!/bin/sh\ntouch {}\nwhile [ ! -f {} ]; do sleep 0.05; done\necho '{{\"entities\":{{\"seen\":[{{\"k\":\"a\",\"v\":\"one\"}}]}}}}'\n",
            started.display(),
            release.display()
        ),
    );
    std::fs::set_permissions(&sync, std::fs::Permissions::from_mode(0o755)).unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let ext = fx
        .svc
        .extension_catalog
        .snapshot(&root)
        .extensions()
        .iter()
        .find(|e| e.name == "acme")
        .cloned()
        .unwrap();
    let spec = ext.collectors[0].clone();
    let hash = crate::collector_runner::approval_hash(&root.join(&ext.path), &spec).unwrap();
    crate::collector_runner::approve(&fx.svc.approvals, "acme", "sync", &hash).unwrap();
}

/// Start the collector through the bus, as a person would, and wait until
/// its program is running.
async fn start(
    fx: &TaskEffortFixture,
) -> tokio::task::JoinHandle<Result<oxplow_domain::CommandOutcome, oxplow_domain::CommandError>> {
    let svc = fx.svc.clone();
    let run = tokio::spawn(async move {
        svc.commands
            .run(
                &Actor::Human,
                "oxplow.collector.sync",
                json!({ "collector": "collector:acme/sync" }),
                false,
            )
            .await
    });
    let started = fx.svc.layout.project_dir.join("started");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the collector's program started");
    run
}

fn release(fx: &TaskEffortFixture) {
    std::fs::write(fx.svc.layout.project_dir.join("release"), "").unwrap();
}

async fn seen(fx: &TaskEffortFixture) -> serde_json::Value {
    serde_json::to_value(
        fx.svc
            .sql
            .query_sql("SELECT count(*) FROM v_acme_seen", vec![], None)
            .await
            .map(|out| out.rows)
            .unwrap_or_default(),
    )
    .unwrap()
}

/// An activation lands while a long collector (an External command) runs,
/// and the collector goes on to record its rows.
#[tokio::test(flavor = "multi_thread")]
async fn an_activation_does_not_wait_for_a_running_collector() {
    let fx = services_with_task_effort().await;
    long_collector(&fx).await;
    let run = start(&fx).await;

    assert!(
        fx.svc.extension_activation.readers().is_empty(),
        "{:?}",
        fx.svc.extension_activation.readers()
    );
    write(
        &fx.svc.layout.project_dir,
        "oxplow/extensions/other/extension.yaml",
        "manifest: 2\nname: other\nintent:\n  purpose: p\n",
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        fx.svc.extension_activation.activate(fx.svc.clone()),
    )
    .await
    .expect("the activation doesn't wait for the collector")
    .expect("it activates");

    release(&fx);
    run.await.unwrap().expect("the collector finishes");
    assert_eq!(seen(&fx).await, json!([[1]]));
}

/// The collector's own extension changes while it runs: what it produced is
/// not published under the old declaration.
#[tokio::test(flavor = "multi_thread")]
async fn a_collector_does_not_publish_across_a_change_to_its_extension() {
    let fx = services_with_task_effort().await;
    long_collector(&fx).await;
    let run = start(&fx).await;

    write(
        &fx.svc.layout.project_dir,
        "oxplow/extensions/acme/extension.yaml",
        &manifest("1.0.1"),
    );
    fx.svc
        .extension_activation
        .activate(fx.svc.clone())
        .await
        .expect("the new version activates");

    release(&fx);
    let error = run.await.unwrap().expect_err("the old run is refused");
    assert!(
        error.to_string().contains("changed or was revoked"),
        "{error}"
    );
    assert_eq!(seen(&fx).await, json!([[0]]));
}

/// Disabling the collector's extension mid-run stops it publishing.
#[tokio::test(flavor = "multi_thread")]
async fn a_collector_does_not_publish_after_its_extension_is_disabled() {
    let fx = services_with_task_effort().await;
    long_collector(&fx).await;
    let run = start(&fx).await;

    write(
        &fx.svc.layout.project_dir,
        ".oxplow/project.yaml",
        "extensions:\n  disabled: [acme]\n",
    );
    fx.svc.reload_config_from_disk().unwrap();
    fx.svc
        .extension_activation
        .activate(fx.svc.clone())
        .await
        .expect("the disabled catalog activates");

    release(&fx);
    let error = run.await.unwrap().expect_err("the run is refused");
    assert!(
        error.to_string().contains("changed or was revoked"),
        "{error}"
    );
    assert_eq!(seen(&fx).await, json!([[0]]));
}

/// An effect's run belongs to its extension's version, not to a catalog
/// generation, and stops being valid when that extension is disabled.
#[tokio::test(flavor = "multi_thread")]
async fn an_effects_run_holds_no_generation_and_ends_with_its_extension() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        "oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nversion: 1.0.0\nintent: { purpose: p, origin: null, examples: [] }\neffects:\n  - id: note\n    summary: s\n    on: [work_item.state_changed]\n    entry: note.star\n",
    );
    write(
        &root,
        "oxplow/extensions/acme/note.star",
        "def transform(x):\n    return {\"skip\": \"noted\"}\n",
    );
    crate::test_fixtures::activate(&fx.svc).await;
    let (ext, decl) = crate::effect_triggers::effects(&fx.svc)
        .into_iter()
        .next()
        .expect("the effect is declared");
    let event = {
        use oxplow_domain::events::schema::{WorkItemStateChanged, WorkItemStateChangedV1};
        let env = oxplow_domain::Envelope::typed::<WorkItemStateChanged>(
            "human",
            &WorkItemStateChangedV1 {
                work_item: oxplow_tasks::work_item_ref(fx.task),
                to: oxplow_domain::work_items::CanonicalState::Done,
            },
        );
        let id = env.id.clone();
        fx.svc.event_log_store.append(env).await.unwrap();
        fx.svc.event_log_store.get(id).await.unwrap().unwrap()
    };
    let run = crate::run_context::for_effect(&fx.svc.extension_catalog, &root, &ext, &decl, &event);
    assert_eq!(run.contribution_ref().unwrap().generation, None);
    run.check_authority().expect("its extension is current");

    write(
        &root,
        ".oxplow/project.yaml",
        "extensions:\n  disabled: [acme]\n",
    );
    fx.svc.reload_config_from_disk().unwrap();
    let error = run
        .check_authority()
        .expect_err("its extension is disabled");
    assert!(error.contains("changed or was revoked"), "{error}");
}
