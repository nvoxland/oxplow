//! One coherent extension generation, through a real project: what a
//! candidate activates, what a failed one keeps, and how readers and
//! publication exclude one another.

use oxplow_domain::Actor;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

/// Extension `name` with one read command, `<name>.answer.get`, whose
/// result is `{"v": v}`; `extra` is appended to its manifest.
fn write_ext(root: &Path, name: &str, v: u32, extra: &str) {
    let dir = root.join(format!(".oxplow/extensions/{name}"));
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
    crate::test_fixtures::activate_with_problems(&svc).await;
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

/// Deploy what the test wrote and activate it, keeping the activation's
/// own outcome (the fixture retries a rejected candidate).
async fn deploy_and_activate(
    svc: &Arc<crate::Services>,
) -> Result<
    Arc<crate::extension_catalog::CatalogSnapshot>,
    oxplow_db::TxError<crate::vocabulary_reactor::VocabularyErrors>,
> {
    let changes = crate::extension_deploy::stage_sources(svc).await;
    svc.extension_activation
        .activate_live(svc.clone(), &changes)
        .await
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
    crate::test_fixtures::activate_with_problems(&svc).await;

    assert_eq!(answer(&svc, "alpha").await, Ok(json!(1)), "the last good");
    assert_eq!(answer(&svc, "beta").await, Ok(json!(7)));
    let held = problems(&svc);
    assert!(
        held.contains("alpha") && held.contains("last working version"),
        "{held}"
    );

    write_ext(dir.path(), "alpha", 3, "");
    crate::test_fixtures::activate_with_problems(&svc).await;
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
    crate::test_fixtures::activate_with_problems(&svc).await;
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
        tokio::spawn(async move { deploy_and_activate(&svc).await })
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

/// A publication waiting on readers can say who they are: each reader is
/// listed with what holds it and for how long, and leaves the list when it
/// lets go.
#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_publication_names_the_readers_it_waits_for() {
    let (dir, svc) = project().await;
    let release = Arc::new(tokio::sync::Notify::new());
    let reader = {
        let (svc, release) = (svc.clone(), release.clone());
        tokio::spawn(async move {
            svc.extension_activation
                .read(async move { release.notified().await })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;

    write_ext(dir.path(), "alpha", 2, "");
    let activating = {
        let svc = svc.clone();
        tokio::spawn(async move { deploy_and_activate(&svc).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !activating.is_finished(),
        "publication waits for the reader"
    );
    let readers = svc.extension_activation.readers();
    assert!(
        readers
            .iter()
            .any(|r| r.held_by.contains("extension_activation_tests")
                && r.held_for >= Duration::from_millis(300)),
        "{readers:?}"
    );

    release.notify_one();
    reader.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(10), activating)
        .await
        .expect("publication goes ahead once the reader is done")
        .unwrap()
        .unwrap();
    assert!(
        svc.extension_activation.readers().is_empty(),
        "{:?}",
        svc.extension_activation.readers()
    );
}

/// A panic after the generation's database state committed doesn't leave
/// the boundary refusing every reader: the activation stages again from
/// what committed and serves it.
#[tokio::test(flavor = "multi_thread")]
async fn a_panic_after_commit_recovers_the_generation() {
    let (dir, svc) = project().await;
    write_ext(dir.path(), "alpha", 2, "");
    svc.extension_activation.panic_once_after_commit();
    crate::test_fixtures::activate_with_problems(&svc).await;
    assert!(svc.extension_activation.pin().await.is_ok());
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
}

/// Disabling an extension withdraws its commands with the next generation.
#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_extension_exposes_nothing() {
    let (dir, svc) = project().await;
    write_ext(dir.path(), "beta", 7, "");
    crate::test_fixtures::activate_with_problems(&svc).await;
    assert_eq!(answer(&svc, "beta").await, Ok(json!(7)));

    std::fs::write(
        dir.path().join(".oxplow/personal.yaml"),
        "disableExtensions: [beta]\n",
    )
    .unwrap();
    svc.reload_config_from_disk().unwrap();
    crate::test_fixtures::activate_with_problems(&svc).await;
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
    std::fs::create_dir_all(dir.path().join(".oxplow/extensions/late/models")).unwrap();
    std::fs::write(
        dir.path()
            .join(".oxplow/extensions/late/models/blocked.sql"),
        "SELECT no_such_column AS ref FROM ref('work_item')",
    )
    .unwrap();
    write_ext(dir.path(), "alpha", 2, "");
    crate::test_fixtures::activate_with_problems(&svc).await;
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
    // Each example is a folder; the folder's README isn't one.
    for entry in std::fs::read_dir(&examples).unwrap() {
        let from = entry.unwrap().path();
        if !from.is_dir() {
            continue;
        }
        let to = dir
            .path()
            .join(".oxplow/extensions")
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
    crate::test_fixtures::activate_with_problems(&svc).await;
    assert_eq!(problems(&svc), "{}");
    let active = svc
        .extension_catalog
        .unpinned(dir.path(), "a test reads declarations");
    for name in ["github", "triage", "prompt-efforts"] {
        assert!(
            active.iter().any(|e| e.name == name && e.enabled),
            "{name} is active"
        );
    }
}

/// A background loop runs while the services do, and `shutdown` stops it.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_the_background_loops() {
    let (_dir, svc) = project().await;
    let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let looping = {
        let ticks = ticks.clone();
        svc.spawn_every(Duration::ZERO, Duration::from_millis(20), move |_svc| {
            let ticks = ticks.clone();
            async move {
                ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(ticks.load(std::sync::atomic::Ordering::SeqCst) >= 2);

    svc.shutdown().await;
    tokio::time::timeout(Duration::from_secs(2), looping)
        .await
        .expect("the loop stops at shutdown")
        .unwrap();
    let after = ticks.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(ticks.load(std::sync::atomic::Ordering::SeqCst), after);
}

/// Two activations at once are serialized: the second waits for the first
/// and both leave one generation, the latest edit's.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_activations_serialize_into_one_generation() {
    let (dir, svc) = project().await;
    write_ext(dir.path(), "alpha", 2, "");
    let changes = crate::extension_deploy::stage_sources(&svc).await;
    let (a, b) = {
        let (s1, s2) = (svc.clone(), svc.clone());
        let (c1, c2) = (changes.clone(), changes.clone());
        (
            tokio::spawn(
                async move { s1.extension_activation.activate_live(s1.clone(), &c1).await },
            ),
            tokio::spawn(
                async move { s2.extension_activation.activate_live(s2.clone(), &c2).await },
            ),
        )
    };
    let (a, b) = (a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
    assert_eq!(a.generation(), b.generation(), "one coherent generation");
    let pin = svc.extension_activation.pin().await.unwrap();
    assert_eq!(pin.generation(), Some(b.generation()));
    drop(pin);
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
}

/// An edit that lands while a candidate is staged (here: waiting on a
/// reader) rejects that publication, and the next one carries the edit.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_during_activation_rejects_it_and_the_next_carries_the_edit() {
    let (dir, svc) = project().await;
    let pin = svc.extension_activation.pin().await.unwrap();
    write_ext(dir.path(), "alpha", 2, "");
    let activating = {
        let svc = svc.clone();
        tokio::spawn(async move { deploy_and_activate(&svc).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!activating.is_finished());
    // A change to which extensions are on overtakes the candidate.
    std::fs::write(
        dir.path().join(".oxplow/personal.yaml"),
        "enableExtensions: [alpha]\ndisableExtensions: [elsewhere]\n",
    )
    .unwrap();
    svc.reload_config_from_disk().unwrap();
    write_ext(dir.path(), "alpha", 3, "");
    drop(pin);
    let outcome = tokio::time::timeout(Duration::from_secs(10), activating)
        .await
        .expect("the staged candidate ends")
        .unwrap();
    assert!(outcome.is_err(), "the stale candidate is rejected");
    assert_eq!(
        answer(&svc, "alpha").await,
        Ok(json!(1)),
        "nothing half-published"
    );

    crate::test_fixtures::activate_with_problems(&svc).await;
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(3)));
}

/// A panic on the staging again too has no third try: the activation fails
/// naming the host, the boundary refuses readers rather than serve maps it
/// can't vouch for, and the next activation brings it back.
#[tokio::test(flavor = "multi_thread")]
async fn a_panic_on_the_restage_too_leaves_the_boundary_unavailable() {
    use crate::extension_activation::PanicPoint;
    let (dir, svc) = project().await;
    write_ext(dir.path(), "alpha", 2, "");
    svc.extension_activation
        .panic_at(PanicPoint::AfterCommit, 2);
    let Err(error) = deploy_and_activate(&svc).await else {
        panic!("two panics are not recovered");
    };
    let text = format!("{error:?}");
    assert!(
        text.contains("<host>") && text.contains("staging it again failed"),
        "{text}"
    );
    let refused = svc
        .extension_activation
        .pin()
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("unavailable"), "{refused}");

    crate::test_fixtures::activate_with_problems(&svc).await;
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
}

/// A panic once the maps are published, before the boundary shows the
/// generation, is recovered the same way.
#[tokio::test(flavor = "multi_thread")]
async fn a_panic_after_the_maps_are_published_recovers_the_generation() {
    use crate::extension_activation::PanicPoint;
    let (dir, svc) = project().await;
    write_ext(dir.path(), "alpha", 2, "");
    svc.extension_activation
        .panic_at(PanicPoint::BeforeBoundary, 1);
    crate::test_fixtures::activate_with_problems(&svc).await;
    assert_eq!(answer(&svc, "alpha").await, Ok(json!(2)));
    assert_eq!(problems(&svc), "{}");
}

/// Reading the activated generation's declarations takes a pin: a reader
/// outside one is a bug the suite fails on, while a pinned read, a pin
/// carried into the blocking pool and the explicit unpinned and live
/// reads are not.
#[tokio::test(flavor = "multi_thread")]
async fn a_generation_read_needs_a_pin_or_says_it_has_none() {
    let (dir, svc) = project().await;
    let root = svc.layout.project_dir.clone();
    let _ = &dir;

    let unpinned = {
        let (svc, root) = (svc.clone(), root.clone());
        tokio::spawn(async move {
            svc.extension_catalog
                .runtime_snapshot(&root)
                .generation()
                .len()
        })
        .await
    };
    assert!(
        unpinned.is_err_and(|e| e.is_panic()),
        "an unpinned runtime read fails the suite"
    );

    let pinned = svc
        .extension_activation
        .read(async {
            svc.extension_catalog
                .runtime_snapshot(&root)
                .generation()
                .len()
        })
        .await
        .unwrap();
    assert!(pinned > 0);

    let catalog = svc.extension_catalog.clone();
    let blocking = svc
        .extension_activation
        .read({
            let root = root.clone();
            async move {
                crate::extension_activation::ExtensionActivation::spawn_blocking(move || {
                    catalog.runtime_snapshot(&root).generation().len()
                })
                .await
            }
        })
        .await
        .unwrap();
    assert_eq!(blocking.unwrap(), pinned, "the blocking pool keeps the pin");

    assert!(!svc.extension_catalog.unpinned(&root, "a test").is_empty());
    assert!(!svc
        .extension_catalog
        .live_snapshot(&root)
        .generation()
        .is_empty());
}
