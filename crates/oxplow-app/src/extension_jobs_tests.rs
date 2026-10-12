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

async fn model() -> (String, oxplow_ai_fake::Seen) {
    oxplow_ai_fake::mock(
        "/chat/completions",
        200,
        json!({"choices": [{"message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 10, "completion_tokens": 5}}),
    )
    .await
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
            Role::Summarize,
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
    svc.jobs.spawn(svc.extension_activation.clone());
}

fn approve(svc: &crate::Services, kind: crate::exec_consent::ProgramKind) {
    let project = svc.layout.project_dir.clone();
    let config = svc.config.read().unwrap().clone();
    let extensions = svc.extension_catalog.get(&project);
    let programs = crate::exec_consent::list(&svc.approvals, &project, &config, &extensions);
    let mut approved = 0;
    for p in programs.into_iter().filter(|p| p.kind == kind) {
        crate::exec_consent::approve_program(
            &svc.approvals,
            &project,
            &config,
            &extensions,
            p.kind,
            &p.name,
            p.version.as_deref().expect("a version to approve"),
        )
        .expect("approved");
        approved += 1;
    }
    assert!(approved > 0, "a {kind:?} program is listed to approve");
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
    json!({
        "job": "summarize",
        "intentKey": intent,
        "request": {
            "schemaVersion": 1,
            "role": "summarize",
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
        let snapshot = run_on(svc, "triage.summary.result", json!({ "id": id })).await;
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
    let id = queued["id"].as_str().unwrap().to_owned();

    let again = run(&fx, "triage.summary.ask", ask("item-1")).await;
    assert_eq!(again["id"], queued["id"], "one intent is one job");
    assert_eq!(again["created"], json!(false));

    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["job"], json!("triage/job/summarize"), "{done}");
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

#[tokio::test(flavor = "multi_thread")]
async fn an_unapproved_job_waits_and_cancelling_it_stops_it() {
    let fx = fixture(false).await;
    let id = run(&fx, "triage.summary.ask", ask("item-2")).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let waiting = run(&fx, "triage.summary.result", json!({ "id": id })).await;
    assert_eq!(waiting["state"], json!("queued"), "{waiting}");
    assert!(fx.seen.lock().unwrap().is_empty(), "nothing was sent");

    let cancelled = run(&fx, "triage.summary.cancel", json!({ "id": id })).await;
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
    let id = run_on(&before, "triage.summary.ask", ask("item-3")).await["id"]
        .as_str()
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
