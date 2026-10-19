//! A job's persistence boundaries under a crash, two schedulers, a full
//! queue, a late result and an extension turned off: each cut at the exact step
//! it is, and what is left to recover from. A crash here is the dispatch
//! panicking (or its transaction aborting) at a [`Stage`]: the durable
//! state is all that remains, which is what a dead process leaves.
//!
//! Driven by hand (`run_one`, `recover_page`) over an idle fixture, so no
//! scheduler races the test.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use oxplow_ai::{
    client::{
        AiError, Cancel, CompleteRequest, Completion, ModelProvider, OnDelta, ProviderInstance,
    },
    fixture::FixtureModel,
    parts::{TypedRequestV1, TypedResponseV1},
};
use oxplow_domain::{extension_job::JobId, Actor};
use serde_json::{json, Value};

use crate::extension_job_dispatch::{
    CrashHook, DispatchError, DispatchOutcome, ExtensionJobDispatcher, Stage,
};
use crate::extension_job_hardening_tests::{expire_lease, observations, state, words};
use crate::extension_job_stream_tests::{
    ask, completions, fixture_idle, manifest_idempotent, reply, rows, slow, Fixture, MANIFEST,
};

fn job_id(id: &str) -> JobId {
    serde_json::from_value(json!(id)).expect("a job id")
}

fn dispatcher(fx: &Fixture, crash: Option<CrashHook>) -> Arc<ExtensionJobDispatcher> {
    fx.svc.jobs.dispatcher_for_test(
        fx.svc.extension_activation.clone(),
        fx.svc.commands.clone(),
        crash,
    )
}

/// A hook that panics the first time `at` is reached.
fn crash_once(at: Stage) -> CrashHook {
    let armed = Arc::new(AtomicBool::new(true));
    Arc::new(move |stage| {
        if stage == at && armed.swap(false, Ordering::SeqCst) {
            panic!("crash at {stage:?}");
        }
    })
}

type Dispatched = Result<Result<DispatchOutcome, DispatchError>, tokio::task::JoinError>;

/// One dispatch on a task of its own, so a crash is a panicked task.
async fn dispatch(d: &Arc<ExtensionJobDispatcher>, id: &str) -> Dispatched {
    let (d, id) = (d.clone(), job_id(id));
    tokio::spawn(async move { d.run_one(id).await }).await
}

async fn recover(d: &ExtensionJobDispatcher) -> (usize, usize) {
    let page = d.recover_page(None, 64).await.expect("recovery runs");
    (page.interrupted, page.requeued)
}

/// The job is cut off at `stage`, its lease runs out and recovery finds
/// it: a `never` job ends `interrupted`, an `idempotent` one runs again
/// and succeeds. Either way its end is announced once, and the crash
/// itself added nothing.
async fn cut_off_at(stage: Stage, idempotent: bool) {
    let manifest = if idempotent {
        manifest_idempotent()
    } else {
        MANIFEST.to_string()
    };
    let fx = fixture_idle(&manifest, vec![reply("the answer")]).await;
    let d = dispatcher(&fx, Some(crash_once(stage)));
    let id = ask(&fx, "crash").await;

    let crashed = dispatch(&d, &id).await;
    match stage {
        Stage::BetweenTransitionAndAnnounce => {
            assert!(matches!(crashed, Ok(Err(_))), "{crashed:?}");
        }
        _ => assert!(crashed.unwrap_err().is_panic()),
    }
    // What is durable: claimed and running, with nothing announced; a
    // crash inside the publish transaction left it unwritten.
    let cut = state(&fx, &id).await;
    assert_eq!(cut["state"], json!("running"), "{cut}");
    assert_eq!(cut["attempt"], json!(1), "{cut}");
    assert_eq!(completions(&fx, &id).await, Vec::<String>::new());

    expire_lease(&fx.svc, &id).await;
    let (interrupted, requeued) = recover(&d).await;
    if idempotent {
        assert_eq!((interrupted, requeued), (0, 1));
        assert_eq!(state(&fx, &id).await["state"], json!("queued"));
        assert_eq!(completions(&fx, &id).await, Vec::<String>::new());
        let again = dispatch(&d, &id).await.unwrap().unwrap();
        assert_eq!(again, DispatchOutcome::Completed);
        let done = state(&fx, &id).await;
        assert_eq!(done["state"], json!("succeeded"), "{done}");
        assert_eq!(done["attempt"], json!(2), "{done}");
        assert_eq!(done["text"], json!("the answer"), "{done}");
        assert_eq!(completions(&fx, &id).await, ["succeeded"]);
        // A round the first attempt finished is read from its record; one
        // it never started is asked now. Never twice.
        assert_eq!(fx.model.asked_typed().len(), 1);
    } else {
        assert_eq!((interrupted, requeued), (1, 0));
        let ended = state(&fx, &id).await;
        assert_eq!(ended["state"], json!("interrupted"), "{ended}");
        assert_eq!(completions(&fx, &id).await, ["interrupted"]);
    }
    assert_eq!(observations(&fx, &id).await, 0);
    // Recovering again finds nothing to do, and announces nothing more.
    assert_eq!(recover(&d).await, (0, 0));
    assert_eq!(completions(&fx, &id).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_never_job_cut_off_after_its_claim_is_interrupted() {
    cut_off_at(Stage::AfterClaim, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idempotent_job_cut_off_after_its_claim_runs_again() {
    cut_off_at(Stage::AfterClaim, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_never_job_cut_off_before_it_publishes_is_interrupted() {
    cut_off_at(Stage::BeforePublish, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idempotent_job_cut_off_before_it_publishes_runs_again_without_asking_twice() {
    cut_off_at(Stage::BeforePublish, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_never_job_cut_off_inside_its_publish_transaction_is_interrupted() {
    cut_off_at(Stage::BetweenTransitionAndAnnounce, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idempotent_job_cut_off_inside_its_publish_transaction_runs_again() {
    cut_off_at(Stage::BetweenTransitionAndAnnounce, true).await;
}

async fn ask_raw(fx: &Fixture, intent: &str, text: &str) -> Result<Value, String> {
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "triage.review.ask",
            json!({
                "job": "review",
                "intentKey": intent,
                "request": {
                    "schemaVersion": 1,
                    "role": "summarize",
                    "messages": [{"role": "user", "parts": [{"type": "text", "text": text}]}],
                },
            }),
            false,
        )
        .await
        .map(|out| out.result)
        .map_err(|e| e.to_string())
}

/// Several people asking for the same intent at once make one job, and
/// exactly one of them created it.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_duplicate_enqueues_make_one_job() {
    let fx = fixture_idle(MANIFEST, vec![reply("unused")]).await;
    let (a, b, c, d) = tokio::join!(
        ask_raw(&fx, "same", "Review the TODOs."),
        ask_raw(&fx, "same", "Review the TODOs."),
        ask_raw(&fx, "same", "Review the TODOs."),
        ask_raw(&fx, "same", "Review the TODOs."),
    );
    let answers: Vec<Value> = [a, b, c, d].into_iter().map(|r| r.unwrap()).collect();
    assert!(answers.iter().all(|a| a["jobRun"] == answers[0]["jobRun"]));
    let created = answers
        .iter()
        .filter(|a| a["created"] == json!(true))
        .count();
    assert_eq!(created, 1, "{answers:?}");
    let count = rows(&fx, "SELECT count(*) FROM v_job_run", vec![]).await;
    assert_eq!(count[0][0], json!(1));
}

/// Two schedulers' dispatchers racing for one queued job: the revision
/// check lets exactly one claim it, so it runs and ends once.
#[tokio::test(flavor = "multi_thread")]
async fn two_dispatchers_over_one_database_claim_a_job_once() {
    let fx = fixture_idle(MANIFEST, vec![reply("once")]).await;
    let (one, two) = (dispatcher(&fx, None), dispatcher(&fx, None));
    let id = ask(&fx, "race").await;
    let (a, b) = tokio::join!(dispatch(&one, &id), dispatch(&two, &id));
    let outcomes = [a.unwrap().unwrap(), b.unwrap().unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == DispatchOutcome::Completed)
            .count(),
        1,
        "{outcomes:?}"
    );
    let done = state(&fx, &id).await;
    assert_eq!(done["state"], json!("succeeded"), "{done}");
    assert_eq!(done["attempt"], json!(1), "{done}");
    assert_eq!(fx.model.asked_typed().len(), 1);
    assert_eq!(completions(&fx, &id).await, ["succeeded"]);
}

/// At the project-wide cap of 1024 unfinished jobs a new intent is refused
/// for capacity, a repeat of one it has is the same job, and the same
/// intent with other input is a conflict, never a capacity error.
#[tokio::test(flavor = "multi_thread")]
async fn the_queue_is_bounded_across_the_project_and_a_changed_duplicate_conflicts() {
    let fx = fixture_idle(MANIFEST, vec![reply("unused")]).await;
    // Another extension's backlog fills most of the project's room.
    fx.svc
        .db
        .transaction(|tx| {
            let mut insert = tx
                .prepare(
                    "INSERT INTO extension_job (
                        id, contribution_id, operation, invoker, run_id,
                        intent_key, replay_policy, state, attempt, fence,
                        created_at_ms, updated_at_ms, revision, request_json)
                     VALUES (?1, 'job:other/backlog', 'ai.complete_typed', 'human',
                        ?1, ?2, 'never', 'queued', 0, 0, 1000, 1000, 1, '{}')",
                )
                .map_err(oxplow_db::map_sql_err)?;
            for n in 0..1000 {
                insert
                    .execute(rusqlite::params![
                        JobId::generate().as_str(),
                        format!("k{n}")
                    ])
                    .map_err(oxplow_db::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    for n in 0..24 {
        ask_raw(&fx, &format!("t{n}"), "Review the TODOs.")
            .await
            .unwrap();
    }
    let total = rows(
        &fx,
        "SELECT count(*) FROM v_job_run WHERE state IN ('queued', 'running')",
        vec![],
    )
    .await;
    assert_eq!(total[0][0], json!(1024));

    let refused = ask_raw(&fx, "one-too-many", "Review the TODOs.")
        .await
        .unwrap_err();
    assert!(refused.contains("capacity"), "{refused}");
    let same = ask_raw(&fx, "t0", "Review the TODOs.").await.unwrap();
    assert_eq!(same["created"], json!(false), "{same}");
    let changed = ask_raw(&fx, "t0", "Review something else.")
        .await
        .unwrap_err();
    assert!(
        changed.contains("conflicts") && !changed.contains("capacity"),
        "{changed}"
    );
}

/// The worker returns after its attempt was fenced by recovery. Its result
/// is kept once, as evidence a model can read; the job stays as recovery
/// ended it, announced once.
#[tokio::test(flavor = "multi_thread")]
async fn a_result_after_lease_recovery_is_one_readable_observation() {
    let fx = fixture_idle(MANIFEST, vec![reply("too late")]).await;
    let (reached, reached_rx) = std::sync::mpsc::channel::<()>();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let hook: CrashHook = Arc::new(move |stage| {
        if stage == Stage::BeforePublish {
            reached.send(()).ok();
            release_rx.lock().unwrap().recv().ok();
        }
    });
    let d = dispatcher(&fx, Some(hook));
    let id = ask(&fx, "late").await;
    let running = tokio::spawn({
        let (d, id) = (d.clone(), job_id(&id));
        async move { d.run_one(id).await }
    });
    tokio::task::spawn_blocking(move || reached_rx.recv())
        .await
        .unwrap()
        .expect("the worker returned");

    expire_lease(&fx.svc, &id).await;
    assert_eq!(recover(&d).await, (1, 0));
    release.send(()).unwrap();
    let outcome = running.await.unwrap().unwrap();
    assert_eq!(outcome, DispatchOutcome::Interrupted);

    assert_eq!(observations(&fx, &id).await, 1);
    let seen = rows(
        &fx,
        "SELECT attempt, outcome, evidence FROM v_job_run_observation WHERE job_id = ?1",
        vec![json!(id)],
    )
    .await;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!((&seen[0][0], &seen[0][1]), (&json!(1), &json!("succeeded")));
    assert!(
        seen[0][2].as_str().unwrap().contains("too late"),
        "{seen:?}"
    );
    assert_eq!(state(&fx, &id).await["state"], json!("interrupted"));
    assert_eq!(completions(&fx, &id).await, ["interrupted"]);
}

/// The extension is turned off between the worker returning and the publish
/// transaction: nothing is published as a success, the job is cancelled
/// with what it streamed, and the result is kept as a late observation.
#[tokio::test(flavor = "multi_thread")]
async fn turning_the_extension_off_between_the_worker_and_the_publish_cancels_the_job() {
    let fx = fixture_idle(MANIFEST, vec![slow(&words(8), 10)]).await;
    let svc = fx.svc.clone();
    let revoked = Arc::new(AtomicBool::new(false));
    let hook: CrashHook = {
        let revoked = revoked.clone();
        Arc::new(move |stage| {
            if stage == Stage::BeforePublish && !revoked.swap(true, Ordering::SeqCst) {
                let svc = svc.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        crate::extension_deploy::set_enabled(&svc, "triage", false, true)
                            .await
                            .unwrap();
                    })
                });
            }
        })
    };
    let d = dispatcher(&fx, Some(hook));
    let id = ask(&fx, "revoked").await;
    let running = tokio::spawn({
        let (d, id) = (d.clone(), job_id(&id));
        async move { d.run_one(id).await }
    });
    // The worker is known to have stopped, so no lease is waited out.
    let outcome = running.await.unwrap().unwrap();
    assert_eq!(outcome, DispatchOutcome::Cancelled);

    let (state, text) = crate::extension_job_hardening_tests::recorded(&fx, &id).await;
    assert_eq!(state, "cancelled");
    assert_eq!(text, words(8));
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
    assert_eq!(observations(&fx, &id).await, 1);
    let kept = rows(
        &fx,
        "SELECT count(*) FROM v_job_run_output WHERE job_id = ?1 AND kind = 'text'",
        vec![json!(id)],
    )
    .await;
    assert!(kept[0][0].as_i64().unwrap() > 0, "the partial output stays");
}

/// A model that, wherever it waits, notes how many write transactions
/// the database has open.
struct DepthProbe {
    inner: Arc<FixtureModel>,
    db: oxplow_db::Database,
    quiet_at_start: Mutex<Option<bool>>,
    during: Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl ModelProvider for DepthProbe {
    fn kind(&self) -> &str {
        oxplow_ai::fixture::KIND
    }
    fn title(&self) -> &str {
        "Probe"
    }
    fn default_base_url(&self) -> Option<&str> {
        Some("fixture:")
    }
    async fn complete(
        &self,
        instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        self.inner.complete(instance, req).await
    }
    async fn complete_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
    ) -> Result<TypedResponseV1, AiError> {
        self.inner.complete_typed(instance, req).await
    }
    async fn stream_typed(
        &self,
        instance: &ProviderInstance<'_>,
        req: &TypedRequestV1,
        cancel: &Cancel,
        on_delta: OnDelta<'_>,
    ) -> Result<TypedResponseV1, AiError> {
        // Where a real model's request would be in flight: a transaction
        // another task opens is gone in a moment; one held across this
        // wait would never be.
        let mut quiet = false;
        for _ in 0..40 {
            if self.db.write_transactions_open() == 0 {
                quiet = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        *self.quiet_at_start.lock().unwrap() = Some(quiet);
        let mut sampled = |delta| {
            self.during
                .lock()
                .unwrap()
                .push(self.db.write_transactions_open());
            on_delta(delta);
        };
        self.inner
            .stream_typed(instance, req, cancel, &mut sampled)
            .await
    }
}

/// No write transaction is held across a model's reply: the database has
/// none open where the model is streaming, which is what lets everything
/// else write meanwhile.
#[tokio::test(flavor = "multi_thread")]
async fn no_write_transaction_is_held_while_a_model_streams() {
    let fx = fixture_idle(MANIFEST, vec![slow(&words(30), 20)]).await;
    let probe = Arc::new(DepthProbe {
        inner: fx.model.clone(),
        db: fx.svc.db.clone(),
        quiet_at_start: Mutex::new(None),
        during: Mutex::new(Vec::new()),
    });
    fx.svc.ai.providers().register_host(probe.clone());
    let d = dispatcher(&fx, None);
    let id = ask(&fx, "depth").await;
    assert_eq!(
        dispatch(&d, &id).await.unwrap().unwrap(),
        DispatchOutcome::Completed
    );

    assert_eq!(*probe.quiet_at_start.lock().unwrap(), Some(true));
    let during = probe.during.lock().unwrap().clone();
    assert!(during.len() >= 30, "{during:?}");
    // A short write by another task can land on a sample; one held across
    // the stream would show on every one.
    assert!(during.contains(&0), "{during:?}");
    assert!(
        during.iter().filter(|open| **open > 0).count() * 2 < during.len(),
        "{during:?}"
    );
}
