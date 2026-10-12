//! One coherent extension generation, through a real project: what a
//! candidate activates, what a failed one keeps, and how readers and
//! publication exclude one another.

use oxplow_domain::Actor;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

/// Extension `name` with one read command, `<name>.answer.get`, whose
/// result is `{"v": v}`; `extra` is appended to its manifest.
fn write_ext(root: &Path, name: &str, v: u32, extra: &str) {
    let dir = root.join(format!("oxplow/extensions/{name}"));
    std::fs::create_dir_all(dir.join("handlers")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        format!(
            "manifest: 2\nname: {name}\nintent:\n  purpose: p\ncommands:\n  - name: answer.get\n    summary: Answer.\n    input_schema: {{ type: object }}\n    entry: handlers/get.star\n    invokers: {{ human: true, agent: true, lens: true }}\n    access: read\n{extra}"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("handlers/get.star"),
        format!("def transform(x):\n    return {{\"commands\": [], \"result\": {{\"v\": {v}}}}}\n"),
    )
    .unwrap();
}

async fn project() -> (tempfile::TempDir, Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    write_ext(dir.path(), "alpha", 1, "");
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("the first candidate activates");
    (dir, svc)
}

async fn answer(svc: &crate::Services, name: &str) -> Result<Value, String> {
    svc.commands
        .run(
            &Actor::Human,
            &format!("{name}.answer.get"),
            json!({}),
            false,
        )
        .await
        .map(|out| out.result["result"]["v"].clone())
        .map_err(|e| e.to_string())
}

fn problems(svc: &crate::Services) -> String {
    format!("{:?}", svc.extension_activation.problems())
}

/// A broken edit doesn't take an extension away: its last working version
/// keeps serving and the problem says so, while a new extension beside it
/// activates.
#[tokio::test(flavor = "multi_thread")]
async fn a_broken_update_keeps_the_last_working_version() {
    let (dir, svc) = project().await;
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(1)));

    write_ext(dir.path(), "alpha", 2, "bogus_key: true\n");
    write_ext(dir.path(), "beta", 7, "");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("the rest activate without alpha's edit");

    assert_eq!(answer(&svc, "alpha").await, Ok(json!(1)), "the last good");
    assert_eq!(answer(&svc, "beta").await, Ok(json!(7)));
    let held = problems(&svc);
    assert!(
        held.contains("alpha") && held.contains("last working version"),
        "{held}"
    );

    write_ext(dir.path(), "alpha", 3, "");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .unwrap();
    assert_eq!(
        answer(&svc, "alpha").await,
        Ok(json!(3)),
        "the fix activates"
    );
    assert_eq!(problems(&svc), "{}", "nothing is held any more");
}

/// A new extension that can't load is never partly exposed.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_broken_extension_exposes_nothing() {
    let (dir, svc) = project().await;
    write_ext(dir.path(), "gamma", 1, "bogus_key: true\n");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .unwrap();
    let err = answer(&svc, "gamma").await.unwrap_err();
    assert!(err.contains("unknown command"), "{err}");
    assert!(
        problems(&svc).contains("isn't active"),
        "{}",
        problems(&svc)
    );
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(1)));
}

/// A run holding the generation keeps it: publication waits for the last
/// reader, and a reader that comes after sees the new generation whole.
#[tokio::test(flavor = "multi_thread")]
async fn publication_waits_for_the_readers_of_the_last_generation() {
    let (dir, svc) = project().await;
    let pin = svc.extension_activation.pin().await.unwrap();
    let before = pin.generation().unwrap().to_owned();

    write_ext(dir.path(), "alpha", 2, "");
    let activating = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.extension_activation.activate(svc.clone()).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !activating.is_finished(),
        "publication waits for the reader"
    );
    assert_eq!(pin.generation(), Some(before.as_str()));

    drop(pin);
    tokio::time::timeout(Duration::from_secs(10), activating)
        .await
        .expect("publication goes ahead once the reader is done")
        .unwrap()
        .unwrap();
    let after = svc.extension_activation.pin().await.unwrap();
    assert_ne!(after.generation(), Some(before.as_str()));
    drop(after);
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
}

/// Disabling an extension withdraws its commands with the next generation.
#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_extension_exposes_nothing() {
    let (dir, svc) = project().await;
    write_ext(dir.path(), "beta", 7, "");
    svc.extension_activation
        .activate(svc.clone())
        .await
        .unwrap();
    assert_eq!(answer(&svc, "beta").await, Ok(json!(7)));

    std::fs::write(
        dir.path().join(".oxplow/project.yaml"),
        "extensions:\n  disabled: [beta]\n",
    )
    .unwrap();
    svc.reload_config_from_disk().unwrap();
    svc.extension_activation
        .activate(svc.clone())
        .await
        .unwrap();
    let err = answer(&svc, "beta").await.unwrap_err();
    assert!(err.contains("unknown command"), "{err}");
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(1)));
}

/// A model whose SQL doesn't compile fails only its own extension: the
/// candidate's other changes still activate.
#[tokio::test(flavor = "multi_thread")]
async fn a_model_that_does_not_compile_holds_back_only_its_extension() {
    let (dir, svc) = project().await;
    write_ext(
        dir.path(),
        "late",
        1,
        "models:\n  - name: blocked\n    version: 1\n    description: Blocked work items.\n    columns:\n      - { name: ref, type: TEXT, doc: Work item ref. }\n",
    );
    std::fs::create_dir_all(dir.path().join("oxplow/extensions/late/models")).unwrap();
    std::fs::write(
        dir.path().join("oxplow/extensions/late/models/blocked.sql"),
        "SELECT no_such_column AS ref FROM ref('work_item')",
    )
    .unwrap();
    write_ext(dir.path(), "alpha", 2, "");
    let outcome = svc.extension_activation.activate(svc.clone()).await;
    assert!(outcome.is_ok(), "{:?}", outcome.err());
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
    assert!(answer(&svc, "late").await.is_err(), "late isn't active");
    assert!(problems(&svc).contains("late"), "{}", problems(&svc));
}

/// The documented examples activate in a project as they are, every one
/// of them, with nothing held back.
#[tokio::test(flavor = "multi_thread")]
async fn the_documented_examples_activate() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
    for entry in std::fs::read_dir(&examples).unwrap() {
        let from = entry.unwrap().path();
        let to = dir
            .path()
            .join("oxplow/extensions")
            .join(from.file_name().unwrap());
        for file in walkdir::WalkDir::new(&from)
            .into_iter()
            .filter_map(Result::ok)
        {
            if file.file_type().is_file() {
                let target = to.join(file.path().strip_prefix(&from).unwrap());
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::copy(file.path(), target).unwrap();
            }
        }
    }
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("activates");
    assert_eq!(problems(&svc), "{}");
    let active = svc.extension_catalog.get(dir.path());
    for name in ["github", "triage", "prompt-efforts"] {
        assert!(
            active.iter().any(|e| e.name == name && e.enabled),
            "{name} is active"
        );
    }
}
