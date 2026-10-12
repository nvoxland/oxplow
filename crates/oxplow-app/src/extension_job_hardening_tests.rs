//! A job's durability under the things that go wrong around it: a lease
//! that runs out, a revoked approval, a changed declaration, a restart, a
//! full queue, a stalled model, and a person cancelling a queued job. All
//! against the in-process fixture model, as `extension_job_stream_tests`.

use std::{sync::Arc, time::Duration};

use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::extension_job_stream_tests::{
    ask, completions, fixture, fixture_idle, prepare, reply, rows, run, slow, until, Fixture,
    LOOKUP, MANIFEST,
};

fn words(count: usize) -> String {
    (0..count)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The job is running and has streamed something.
async fn streaming(fx: &Fixture, id: &str) -> Value {
    until(fx, id, |s| {
        s["state"] == json!("running") && !s["text"].as_str().unwrap_or_default().is_empty()
    })
    .await
}

async fn state(fx: &Fixture, id: &str) -> Value {
    run(
        fx,
        &Actor::Human,
        "triage.review.result",
        json!({ "id": id }),
    )
    .await
}

/// Make the running attempt's lease a thing of the past, as if oxplow had
/// been gone for longer than it lasts.
async fn expire_lease(svc: &crate::Services, id: &str) {
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

async fn observations(fx: &Fixture, id: &str) -> i64 {
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

/// The key the job's approval is recorded under.
fn job_key(fx: &Fixture) -> String {
    let project = fx.svc.layout.project_dir.clone();
    let config = fx.svc.config.read().unwrap().clone();
    let extensions = fx.svc.extension_catalog.get(&project);
    crate::exec_consent::list(&fx.svc.approvals, &project, &config, &extensions)
        .into_iter()
        .find(|p| p.kind == crate::exec_consent::ProgramKind::Job)
        .expect("the job is a program")
        .key()
}

/// An attempt whose lease ran out ends `interrupted` — nothing is replayed
/// and nothing the stopped worker says later changes that.
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

/// Revoking the job's approval while it streams stops it: it ends
/// `cancelled`, keeping the partial reply.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_approval_stops_a_streaming_job() {
    let fx = fixture(MANIFEST, vec![slow(&words(200), 20)]).await;
    let id = ask(&fx, "revoke").await;
    streaming(&fx, &id).await;

    assert!(fx.svc.approvals.revoke(&job_key(&fx)).unwrap());
    let done = until(&fx, &id, |s| s["state"] != json!("running")).await;
    assert_eq!(done["state"], json!("cancelled"), "{done}");
    let text = done["result"]["parts"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.starts_with("w0 ") && text.len() < words(200).len(),
        "{done}"
    );
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
}

/// A declaration changed (and re-approved) while the job streams: the
/// activation stops it, and what it had is not published as the old
/// declaration's success.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_does_not_succeed_across_a_generation_change() {
    let fx = fixture(MANIFEST, vec![slow(&words(40), 50)]).await;
    let id = ask(&fx, "generation").await;
    streaming(&fx, &id).await;

    let changed = MANIFEST.replace("version: 1", "version: 2");
    std::fs::write(
        fx._dir
            .path()
            .join("oxplow/extensions/triage/extension.yaml"),
        changed,
    )
    .unwrap();
    crate::exec_consent::approve_every(&fx.svc, crate::exec_consent::ProgramKind::Job).unwrap();
    fx.svc
        .extension_activation
        .activate(fx.svc.clone())
        .await
        .expect("the new declaration activates");

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
/// worker left and interrupts it once its lease is out; nothing replays.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_runs_what_was_queued_and_interrupts_what_was_running() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join("oxplow/extensions/triage");
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
    let count = rows(&fx, "SELECT count(*) FROM v_job", vec![]).await;
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
    run(&fx, &Actor::Human, "oxplow.job.cancel", json!({ "id": id })).await;
    let after = state(&fx, &id).await;
    assert_eq!(after["state"], json!("cancelled"), "{after}");
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
    assert!(fx.model.asked_typed().is_empty());
}
