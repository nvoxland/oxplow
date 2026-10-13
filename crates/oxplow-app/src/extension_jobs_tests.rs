//! A declared job end to end: an extension's command records intent, the
//! scheduler runs the typed AI operation outside SQL once a person approved
//! the job, and the extension reads its recorded result back.

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_domain::Actor;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

const MANIFEST: &str = "manifest: 2
name: triage
intent:
  purpose: p
ai_profiles:
  - { id: sort, description: Sorts items., fallback: summarize }
jobs:
  - { id: summarize, description: Summarize an item., version: 1, operation: ai.complete_typed, needs: [ai.compute] }
commands:
  - { name: summary.ask, summary: Ask for a summary., scope: jobs.write, op: enqueue, invokers: { human: true, agent: true, lens: true } }
  - { name: summary.cancel, summary: Stop asking., scope: jobs.write, op: cancel, invokers: { human: true, agent: true, lens: true } }
  - { name: summary.result, summary: Read the answer., scope: jobs.read, op: snapshot, invokers: { human: true, agent: true, lens: true } }
";

struct Fixture {
    _dir: tempfile::TempDir,
    svc: Arc<crate::Services>,
    seen: oxplow_ai_fake::Seen,
}

/// A project with the `triage` extension, a mock OpenAI-compatible model on
/// the summarize role and every AI provider approved; `approve_jobs` also
/// approves its job, as a person does in Settings → Data → Programs.
async fn fixture(approve_jobs: bool) -> Fixture {
    let dir = project();
    let (base, seen) = model().await;
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    configure(&svc, base);
    start(&svc, approve_jobs).await;
    Fixture {
        _dir: dir,
        svc,
        seen,
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join("oxplow/extensions/triage");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("extension.yaml"), MANIFEST).unwrap();
    dir
}

/// An OpenAI-compatible model that streams "hello" (a job's rounds stream).
async fn model() -> (String, oxplow_ai_fake::Seen) {
    let event = |v: Value| format!("data: {v}\n\n");
    let (base, seen, _) = oxplow_ai_fake::mock_sse(
        "/chat/completions",
        vec![
            event(json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "hel"}}]})),
            event(json!({"choices": [{"index": 0, "delta": {"content": "lo"}, "finish_reason": "stop"}]})),
            event(json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5}})),
            "data: [DONE]\n\n".into(),
        ],
        Duration::from_millis(5),
        Duration::ZERO,
    )
    .await;
    (base, seen)
}

fn configure(svc: &crate::Services, base: String) {
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "a".into(),
                kind: "openai_compatible".into(),
                base_url: Some(base),
            },
            Some("sk-1".into()),
        )
        .unwrap();
    svc.ai
        .set_role(
            &Role::SUMMARIZE,
            Some(RoleBinding {
                provider: "a".into(),
                model: "m".into(),
            }),
        )
        .unwrap();
    crate::test_fixtures::approve_ai_providers(svc);
}

/// Activate the project's extensions and start the job scheduler, as boot does.
async fn start(svc: &Arc<crate::Services>, approve_jobs: bool) {
    if approve_jobs {
        approve(svc, crate::exec_consent::ProgramKind::Job);
    }
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("the extension activates");
    let problems = svc.extension_activation.problems();
    assert!(problems.is_empty(), "{problems:?}");
    svc.jobs
        .spawn(svc.extension_activation.clone(), svc.commands.clone());
}

fn approve(svc: &crate::Services, kind: crate::exec_consent::ProgramKind) {
    let approved = crate::exec_consent::approve_every(svc, kind).expect("approved");
    assert!(
        !approved.is_empty(),
        "a {kind:?} program is listed to approve"
    );
}

async fn run(fx: &Fixture, command: &str, input: Value) -> Value {
    run_on(&fx.svc, command, input).await
}

async fn run_on(svc: &crate::Services, command: &str, input: Value) -> Value {
    svc.commands
        .run(&Actor::Human, command, input, false)
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"))
        .result
}

fn ask(intent: &str) -> Value {
    ask_as(intent, "summarize")
}

fn ask_as(intent: &str, role: &str) -> Value {
    json!({
        "job": "summarize",
        "intentKey": intent,
        "request": {
            "schemaVersion": 1,
            "role": role,
            "messages": [{"role": "user", "parts": [{"type": "text", "text": "Summarize: the build is red."}]}],
        },
    })
}

/// Polls the job until `done` holds of its snapshot, up to ten seconds.
async fn until(fx: &Fixture, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    until_on(&fx.svc, id, done).await
}

async fn until_on(svc: &crate::Services, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = run_on(
            svc,
            "triage.summary.result",
            json!({ "job_run": oxplow_domain::refs::build::job_run_ref(id) }),
        )
        .await;
        if done(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the job never got there: {snapshot}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_job_runs_and_its_result_is_read_back() {
    let fx = fixture(true).await;
    let queued = run(&fx, "triage.summary.ask", ask("item-1")).await;
    assert_eq!(queued["created"], json!(true), "{queued}");
    let id = oxplow_domain::refs::build::id_of(queued["jobRun"].as_str().unwrap(), "job_run")
        .unwrap()
        .to_owned();

    let again = run(&fx, "triage.summary.ask", ask("item-1")).await;
    assert_eq!(again["jobRun"], queued["jobRun"], "one intent is one job");
    assert_eq!(again["created"], json!(false));

    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["job"], json!("job:triage/summarize"), "{done}");
    assert!(
        done["result"].to_string().contains("hello"),
        "the model's answer is the result: {done}"
    );
    assert_eq!(fx.seen.lock().unwrap().len(), 1, "one model call");

    let completed: i64 = fx
        .svc
        .db
        .read(|conn| {
            conn.query_row(
                "SELECT count(*) FROM event_log WHERE type = 'contribution.job.completed'",
                [],
                |row| row.get(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    assert_eq!(completed, 1, "its completion is an event");
}

/// A job names one of its extension's AI profiles by its bare id: it runs
/// on the profile's model (here its fallback's) and the call records the
/// qualified profile. A name the extension doesn't declare is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_runs_on_its_extensions_ai_profile() {
    let fx = fixture(true).await;
    let id = oxplow_domain::refs::build::id_of(
        run(&fx, "triage.summary.ask", ask_as("item-p", "sort")).await["jobRun"]
            .as_str()
            .unwrap(),
        "job_run",
    )
    .unwrap()
    .to_owned();
    until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    let roles = fx
        .svc
        .sql
        .query_sql("SELECT role FROM v_ai_call", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&roles.rows).unwrap(),
        json!([["triage/sort"]])
    );
    let err = fx
        .svc
        .commands
        .run(
            &Actor::Human,
            "triage.summary.ask",
            ask_as("item-q", "review"),
            false,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("`role` is `main`, `summarize`, `decide` or one of this extension's `ai_profiles` (`sort`)"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unapproved_job_waits_and_cancelling_it_stops_it() {
    let fx = fixture(false).await;
    let id = oxplow_domain::refs::build::id_of(
        run(&fx, "triage.summary.ask", ask("item-2")).await["jobRun"]
            .as_str()
            .unwrap(),
        "job_run",
    )
    .unwrap()
    .to_owned();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let waiting = run(
        &fx,
        "triage.summary.result",
        json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&id) }),
    )
    .await;
    assert_eq!(waiting["state"], json!("queued"), "{waiting}");
    assert!(fx.seen.lock().unwrap().is_empty(), "nothing was sent");

    let cancelled = run(
        &fx,
        "triage.summary.cancel",
        json!({ "job_run": oxplow_domain::refs::build::job_run_ref(&id) }),
    )
    .await;
    assert_eq!(cancelled["cancellationRequested"], json!(true));
    let done = until(&fx, &id, |s| s["state"] == json!("cancelled")).await;
    assert_eq!(done["cancellationRequested"], json!(true), "{done}");
    assert!(fx.seen.lock().unwrap().is_empty(), "nothing was sent");
}

/// Intent is durable: a job recorded before oxplow stopped runs after it
/// starts again, once a person approves it.
#[tokio::test(flavor = "multi_thread")]
async fn a_recorded_job_runs_after_a_restart() {
    let dir = project();
    let global = dir.path().join(".oxplow/global-config");
    let secrets: Arc<dyn oxplow_ai::secrets::SecretStore> =
        Arc::new(oxplow_ai::secrets::MemorySecrets::default());
    let open = || async {
        {
            crate::Services::on_machine_with_database(
                dir.path(),
                global.clone(),
                secrets.clone(),
                crate::providers::host::process_env(),
                oxplow_db::Database::open(dir.path().join("restart.sqlite")).unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let (base, seen) = model().await;

    let before = open().await;
    configure(&before, base);
    start(&before, false).await;
    let id = oxplow_domain::refs::build::id_of(
        run_on(&before, "triage.summary.ask", ask("item-3")).await["jobRun"]
            .as_str()
            .unwrap(),
        "job_run",
    )
    .unwrap()
    .to_owned();
    before.jobs.shutdown();
    drop(before);

    let after = open().await;
    start(&after, true).await;
    let done = until_on(&after, &id, |s| s["state"] == json!("succeeded")).await;
    assert!(done["result"].to_string().contains("hello"), "{done}");
    assert_eq!(seen.lock().unwrap().len(), 1, "one model call");
}
