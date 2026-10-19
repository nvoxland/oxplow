//! A job's durability under the things that go wrong around it: a lease
//! that runs out, an extension turned off, a changed declaration, a restart, a
//! full queue, a stalled model, and a person cancelling a queued job. All
//! against the in-process fixture model, as `extension_job_stream_tests`.

use std::{sync::Arc, time::Duration};

use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::extension_job_stream_tests::{
    ask, calls, completions, fixture, fixture_idle, manifest_idempotent, prepare, reply, rows, run,
    slow, until, Fixture, LOOKUP, MANIFEST,
};

pub(super) fn words(count: usize) -> String {
    (0..count)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The job is running and has streamed something.
pub(super) async fn streaming(fx: &Fixture, id: &str) -> Value {
    until(fx, id, |s| {
        s["state"] == json!("running") && !s["text"].as_str().unwrap_or_default().is_empty()
    })
    .await
}

pub(super) async fn state(fx: &Fixture, id: &str) -> Value {
    run(
        fx,
        &Actor::Human,
        "triage.review.result",
        json!({ "job_run": oxplow_domain::refs::build::job_run_ref(id) }),
    )
    .await
}

/// A job's state and the text it streamed, read from the models: the
/// extension's commands are gone while it is off.
pub(super) async fn recorded(fx: &Fixture, id: &str) -> (String, String) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = rows(
            fx,
            "SELECT state FROM v_job_run WHERE id = ?1",
            vec![json!(id)],
        )
        .await;
        let state = state[0][0].as_str().unwrap_or_default().to_string();
        if state != "running" {
            let text = rows(
                fx,
                "SELECT COALESCE(group_concat(text, ''), '') FROM
                   (SELECT text FROM v_job_run_output WHERE job_id = ?1 AND kind = 'text' ORDER BY seq)",
                vec![json!(id)],
            )
            .await;
            return (state, text[0][0].as_str().unwrap_or_default().to_string());
        }
        assert!(tokio::time::Instant::now() < deadline, "still {state}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Make the running attempt's lease a thing of the past, as if oxplow had
/// been gone for longer than it lasts.
pub(super) async fn expire_lease(svc: &crate::Services, id: &str) {
    let id = id.to_owned();
    svc.db
        .transaction(move |tx| {
            let lease: i64 = tx
                .query_row(
                    "SELECT started_at_ms + 1 FROM extension_job_attempt
                      WHERE job_id = ?1 AND closed_at_ms IS NULL",
                    [&id],
                    |row| row.get(0),
                )
                .map_err(oxplow_db::map_sql_err)?;
            // The store never shortens a lease; only the passing of time does,
            // so the attempt's guard is taken off this throwaway database.
            tx.execute_batch("DROP TRIGGER IF EXISTS extension_job_attempt_immutable")
                .map_err(oxplow_db::map_sql_err)?;
            tx.execute(
                "UPDATE extension_job_attempt SET lease_until_ms = ?2
                  WHERE job_id = ?1 AND closed_at_ms IS NULL",
                rusqlite::params![&id, lease],
            )
            .map_err(oxplow_db::map_sql_err)?;
            tx.execute(
                "UPDATE extension_job SET lease_until_ms = ?2, revision = revision + 1
                  WHERE id = ?1",
                rusqlite::params![&id, lease],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .expect("the lease is forced");
}

pub(super) async fn observations(fx: &Fixture, id: &str) -> i64 {
    let id = id.to_owned();
    fx.svc
        .db
        .transaction(move |tx| {
            tx.query_row(
                "SELECT count(*) FROM extension_job_observation WHERE job_id = ?1",
                [&id],
                |row| row.get(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .expect("observations are counted")
}

/// A `never` job whose lease ran out ends `interrupted` — it isn't
/// replayed, and nothing the stopped worker says later changes that.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_whose_lease_ran_out_is_interrupted() {
    let fx = fixture(MANIFEST, vec![slow(&words(100), 50)]).await;
    let id = ask(&fx, "lease").await;
    streaming(&fx, &id).await;

    expire_lease(&fx.svc, &id).await;
    until(&fx, &id, |s| s["state"] == json!("interrupted")).await;
    assert_eq!(completions(&fx, &id).await, ["interrupted"]);

    // The worker was told to stop and cannot close or add to the attempt.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let later = state(&fx, &id).await;
    assert_eq!(later["state"], json!("interrupted"), "{later}");
    assert_eq!(completions(&fx, &id).await, ["interrupted"]);
}

/// Turning the job's extension off while it streams stops it: it ends
/// `cancelled`, keeping the partial reply.
#[tokio::test(flavor = "multi_thread")]
async fn turning_the_extension_off_stops_a_streaming_job() {
    let fx = fixture(MANIFEST, vec![slow(&words(200), 20)]).await;
    let id = ask(&fx, "revoke").await;
    streaming(&fx, &id).await;

    crate::extension_deploy::set_enabled(&fx.svc, "triage", false, true)
        .await
        .unwrap();
    let (state, text) = recorded(&fx, &id).await;
    assert_eq!(state, "cancelled");
    assert!(
        text.starts_with("w0 ") && text.len() < words(200).len(),
        "{text}"
    );
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
}

/// Add an extension of no concern to the job and activate the catalog
/// with it.
async fn change_another_extension(fx: &Fixture) {
    let other = fx._dir.path().join(".oxplow/extensions/other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("extension.yaml"),
        "manifest: 2\nname: other\nintent:\n  purpose: p\n",
    )
    .unwrap();
    crate::test_fixtures::activate(&fx.svc).await;
}

/// A job belongs to its extension's version, not to the whole catalog: one
/// queued before another extension changed still runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_job_runs_after_another_extension_changes() {
    let fx = fixture_idle(MANIFEST, vec![reply("still mine")]).await;
    let id = ask(&fx, "unrelated").await;
    change_another_extension(&fx).await;

    fx.svc
        .jobs
        .spawn(fx.svc.extension_activation.clone(), fx.svc.commands.clone());
    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["text"], json!("still mine"), "{done}");
    assert_eq!(completions(&fx, &id).await, ["succeeded"]);
}

/// A running job holds no catalog: another extension's change activates
/// while it streams, and the job goes on to succeed.
#[tokio::test(flavor = "multi_thread")]
async fn an_activation_does_not_wait_for_a_running_job() {
    let fx = fixture(MANIFEST, vec![slow(&words(60), 50)]).await;
    let id = ask(&fx, "unrelated").await;
    streaming(&fx, &id).await;

    tokio::time::timeout(Duration::from_secs(2), change_another_extension(&fx))
        .await
        .expect("the activation doesn't wait for the job");
    assert_eq!(state(&fx, &id).await["state"], json!("running"));
    let done = until(&fx, &id, |s| s["state"] != json!("running")).await;
    assert_eq!(done["state"], json!("succeeded"), "{done}");
    assert_eq!(done["text"], json!(words(60)), "{done}");
}

/// A declaration changed while the job streams: the
/// activation stops it, and what it had is not published as the old
/// declaration's success.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_does_not_succeed_across_a_change_to_its_declaration() {
    let fx = fixture(MANIFEST, vec![slow(&words(40), 50)]).await;
    let id = ask(&fx, "generation").await;
    streaming(&fx, &id).await;

    let changed = MANIFEST.replace("version: 1", "version: 2");
    std::fs::write(
        fx._dir
            .path()
            .join(".oxplow/extensions/triage/extension.yaml"),
        changed,
    )
    .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;

    let done = until(&fx, &id, |s| s["state"] != json!("running")).await;
    assert_eq!(done["state"], json!("cancelled"), "{done}");
    assert_eq!(done["version"], json!(1), "it stays the old declaration's");
    assert!(
        done["text"].as_str().unwrap().len() < words(40).len(),
        "{done}"
    );
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
    assert_eq!(observations(&fx, &id).await, 0);
}

/// A restart finds a queued job and runs it, and a running one with no
/// worker left and interrupts it once its lease is out; a `never` job
/// isn't replayed.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_runs_what_was_queued_and_interrupts_what_was_running() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/triage");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(ext.join("extension.yaml"), MANIFEST).unwrap();
    std::fs::write(ext.join("commands/lookup.star"), LOOKUP).unwrap();
    let global = dir.path().join(".oxplow/global-config");
    let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
        Arc::new(oxplow_ai::secrets::MemorySecrets::default());
    let open = || async {
        crate::Services::on_machine_with_database(
            dir.path(),
            global.clone(),
            secrets.clone(),
            crate::providers::host::process_env(),
            oxplow_db::Database::open(dir.path().join("restart.sqlite")).unwrap(),
        )
        .await
        .unwrap()
    };

    let before = open().await;
    let model = prepare(&before, vec![slow(&words(400), 50)]).await;
    before
        .jobs
        .spawn(before.extension_activation.clone(), before.commands.clone());
    let fx = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        svc: before,
        model,
    };
    let running = ask(&fx, "running").await;
    streaming(&fx, &running).await;
    fx.svc.jobs.shutdown();
    let queued = ask(&fx, "queued").await;
    drop(fx);

    let svc = open().await;
    let model = prepare(&svc, vec![reply("ran after the restart")]).await;
    let fx = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        svc,
        model,
    };
    expire_lease(&fx.svc, &running).await;
    fx.svc
        .jobs
        .spawn(fx.svc.extension_activation.clone(), fx.svc.commands.clone());
    let done = until(&fx, &queued, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["text"], json!("ran after the restart"), "{done}");
    until(&fx, &running, |s| s["state"] == json!("interrupted")).await;
    assert_eq!(completions(&fx, &running).await, ["interrupted"]);
    assert_eq!(
        fx.model.asked_typed().len(),
        1,
        "the interrupted job isn't replayed"
    );
}

/// An extension's 257th unfinished job is refused; asking again for one it
/// already has isn't a new job.
#[tokio::test(flavor = "multi_thread")]
async fn the_queue_is_bounded_per_extension_and_a_duplicate_is_not_new() {
    let fx = fixture_idle(MANIFEST, vec![reply("unused")]).await;
    let first = ask(&fx, "i0").await;
    for i in 1..256 {
        ask(&fx, &format!("i{i}")).await;
    }
    let refused = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            "triage.review.ask",
            json!({
                "job": "review",
                "intentKey": "i256",
                "request": {
                    "schemaVersion": 1,
                    "role": "summarize",
                    "messages": [{"role": "user", "parts": [{"type": "text", "text": "Review the TODOs."}]}],
                },
            }),
            false,
        )
        .await
        .expect_err("the queue is full");
    assert!(refused.to_string().contains("capacity"), "{refused}");
    assert_eq!(
        ask(&fx, "i0").await,
        first,
        "the same intent is the same job"
    );
    let count = rows(&fx, "SELECT count(*) FROM v_job_run", vec![]).await;
    assert_eq!(count[0][0], json!(256));
}

/// A reply stalled on the wire holds no database transaction: an unrelated
/// audited command goes through at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_stream_holds_no_write_transaction() {
    let fx = fixture(MANIFEST, vec![slow(&words(100), 300)]).await;
    let id = ask(&fx, "stall").await;
    streaming(&fx, &id).await;
    for _ in 0..3 {
        let started = std::time::Instant::now();
        run(
            &fx,
            &Actor::Human,
            "oxplow.config.set",
            json!({ "key": "metricRetentionDays", "value": 30 }),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "the command waited {:?}",
            started.elapsed()
        );
    }
    assert_eq!(state(&fx, &id).await["state"], json!("running"));
}

/// A person cancelling a queued job ends it at once, with its completion.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_queued_job_ends_it_at_once() {
    let fx = fixture_idle(MANIFEST, vec![reply("unused")]).await;
    let id = ask(&fx, "queued").await;
    assert_eq!(state(&fx, &id).await["state"], json!("queued"));
    run(
        &fx,
        &Actor::Human,
        "oxplow.job_run.cancel",
        json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&id) }),
    )
    .await;
    let after = state(&fx, &id).await;
    assert_eq!(after["state"], json!("cancelled"), "{after}");
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
    assert!(fx.model.asked_typed().is_empty());
}

/// An `idempotent` job whose lease ran out runs again, as its next
/// attempt: the round it had finished is read back from its record, the
/// cut-off one is asked again, and its end is announced once, when it
/// really ends.
#[tokio::test(flavor = "multi_thread")]
async fn an_idempotent_job_whose_lease_ran_out_runs_again() {
    let fx = fixture(
        &manifest_idempotent(),
        vec![
            calls(&[("todos_lookup", json!({ "path": "src/a.rs" }))]),
            slow(&words(100), 50),
            reply("done after the retry"),
        ],
    )
    .await;
    let id = ask(&fx, "replayed").await;
    // Its first round is done (the tool ran) and its second streams.
    streaming(&fx, &id).await;
    assert_eq!(fx.model.asked_typed().len(), 2);

    expire_lease(&fx.svc, &id).await;
    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["attempt"], json!(2), "{done}");
    assert_eq!(done["text"], json!("done after the retry"), "{done}");
    assert_eq!(completions(&fx, &id).await, ["succeeded"]);
    // Round one came from its record; only the cut-off round was asked
    // again.
    assert_eq!(fx.model.asked_typed().len(), 3);
    let second = rows(
        &fx,
        "SELECT count(*) FROM v_job_run_output WHERE job_id = ?1 AND attempt = 2 AND kind = 'tool_result'",
        vec![json!(id)],
    )
    .await;
    assert_eq!(second, [[json!(1)]], "the tool ran again on attempt 2");
}

/// An `idempotent` job cut off on each of its three attempts ends
/// `interrupted` after the third, announced then and only then.
#[tokio::test(flavor = "multi_thread")]
async fn an_idempotent_job_interrupted_three_times_ends_interrupted() {
    let fx = fixture(&manifest_idempotent(), vec![slow(&words(400), 50)]).await;
    let id = ask(&fx, "thrice").await;
    for attempt in 1..=3 {
        until(&fx, &id, |s| {
            s["state"] == json!("running")
                && s["attempt"] == json!(attempt)
                && !s["text"].as_str().unwrap_or_default().is_empty()
        })
        .await;
        expire_lease(&fx.svc, &id).await;
        if attempt < 3 {
            until(&fx, &id, |s| s["attempt"] == json!(attempt + 1)).await;
            assert_eq!(completions(&fx, &id).await, Vec::<String>::new());
        }
    }
    let ended = until(&fx, &id, |s| s["state"] == json!("interrupted")).await;
    assert_eq!(ended["attempt"], json!(3), "{ended}");
    assert_eq!(completions(&fx, &id).await, ["interrupted"]);
}

/// A restart replays an `idempotent` job that was running: once its lease
/// is out it runs again and succeeds, its end announced once.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_replays_an_idempotent_job_that_was_running() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/triage");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(ext.join("extension.yaml"), manifest_idempotent()).unwrap();
    std::fs::write(ext.join("commands/lookup.star"), LOOKUP).unwrap();
    let global = dir.path().join(".oxplow/global-config");
    let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
        Arc::new(oxplow_ai::secrets::MemorySecrets::default());
    let open = || async {
        crate::Services::on_machine_with_database(
            dir.path(),
            global.clone(),
            secrets.clone(),
            crate::providers::host::process_env(),
            oxplow_db::Database::open(dir.path().join("restart.sqlite")).unwrap(),
        )
        .await
        .unwrap()
    };

    let before = open().await;
    let model = prepare(&before, vec![slow(&words(400), 50)]).await;
    before
        .jobs
        .spawn(before.extension_activation.clone(), before.commands.clone());
    let fx = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        svc: before,
        model,
    };
    let running = ask(&fx, "running").await;
    streaming(&fx, &running).await;
    fx.svc.jobs.shutdown();
    drop(fx);

    let svc = open().await;
    let model = prepare(&svc, vec![reply("ran again after the restart")]).await;
    let fx = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        svc,
        model,
    };
    expire_lease(&fx.svc, &running).await;
    fx.svc
        .jobs
        .spawn(fx.svc.extension_activation.clone(), fx.svc.commands.clone());
    let done = until(&fx, &running, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["attempt"], json!(2), "{done}");
    assert_eq!(done["text"], json!("ran again after the restart"), "{done}");
    assert_eq!(completions(&fx, &running).await, ["succeeded"]);
    // The cut-off round was never recorded, so it was asked again.
    assert_eq!(fx.model.asked_typed().len(), 1);
}
