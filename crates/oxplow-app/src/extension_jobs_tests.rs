//! A declared job end to end: an extension's command records intent, the
//! scheduler runs the typed AI operation outside SQL for
//! the job, and the extension reads its recorded result back.

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_domain::Actor;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

const MANIFEST: &str = "manifest: 2
name: triage
intent:
  purpose: p
permissions: { sql: read, ai: true }
ai_profiles:
  - { id: sort, description: Sorts items., fallback: summarize }
jobs:
  - { id: summarize, description: Summarize an item., version: 1, operation: ai.complete_typed, needs: [ai.compute] }
  - { id: lookup, description: Look an item up in oxplow., version: 1, operation: ai.complete_typed, needs: [ai.compute, sql.read] }
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
/// the summarize role and the job scheduler running.
async fn fixture() -> Fixture {
    let dir = project();
    let (base, seen) = model().await;
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    configure(&svc, base);
    start(&svc, true).await;
    Fixture {
        _dir: dir,
        svc,
        seen,
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/triage");
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
        .set_role(&Role::SUMMARIZE, Some(RoleBinding::new("a", "m")))
        .unwrap();
}

/// Activate the project's extensions and, with `scheduler`, start the job
/// scheduler, as boot does.
async fn start(svc: &Arc<crate::Services>, scheduler: bool) {
    crate::test_fixtures::activate(svc).await;
    let problems = svc.extension_activation.problems();
    assert!(problems.is_empty(), "{problems:?}");
    if scheduler {
        svc.jobs
            .spawn(svc.extension_activation.clone(), svc.commands.clone());
    }
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
            json!({ "job_run": oxplow_domain::uri::build::job_run_uri(id) }),
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
async fn a_job_runs_and_its_result_is_read_back() {
    let fx = fixture().await;
    let queued = run(&fx, "triage.summary.ask", ask("item-1")).await;
    assert_eq!(queued["created"], json!(true), "{queued}");
    let id = oxplow_domain::uri::build::id_of(
        queued["jobRun"].as_str().unwrap(),
        oxplow_domain::uri::resources::JOB_RUN,
    )
    .unwrap()
    .to_owned();

    let again = run(&fx, "triage.summary.ask", ask("item-1")).await;
    assert_eq!(again["jobRun"], queued["jobRun"], "one intent is one job");
    assert_eq!(again["created"], json!(false));

    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(
        done["job"],
        json!("oxplow:oxplow/job?job=triage/summarize"),
        "{done}"
    );
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

/// A person's restriction on a binding holds whichever job runs on it: a
/// job offered `query_sql` fails on a binding restricted from SQL, naming
/// the binding and the restriction, before any model call; a job asking
/// nothing it forbids runs on the same binding.
#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_binding_fails_a_job_needing_what_it_forbids() {
    let fx = fixture().await;
    fx.svc
        .ai
        .set_role(
            &Role::SUMMARIZE,
            Some(RoleBinding {
                restrict: [oxplow_domain::ai::AiRestriction::Sql].into(),
                ..RoleBinding::new("a", "m")
            }),
        )
        .unwrap();
    let job_run = |v: Value| {
        oxplow_domain::uri::build::id_of(
            v["jobRun"].as_str().unwrap(),
            oxplow_domain::uri::resources::JOB_RUN,
        )
        .unwrap()
        .to_owned()
    };
    let mut lookup = ask("item-sql");
    lookup["job"] = json!("lookup");
    let id = job_run(run(&fx, "triage.summary.ask", lookup).await);
    let failed = until(&fx, &id, |s| s["state"] == json!("failed")).await;
    assert!(
        failed
            .to_string()
            .contains("the `summarize` role's binding `a · m` is restricted: no SQL"),
        "{failed}"
    );
    assert!(fx.seen.lock().unwrap().is_empty(), "no model call");

    let id = job_run(run(&fx, "triage.summary.ask", ask("item-plain")).await);
    until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
}

/// A job names one of its extension's AI profiles by its bare id: it runs
/// on the profile's model (here its fallback's) and the call records the
/// qualified profile. A name the extension doesn't declare is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_runs_on_its_extensions_ai_profile() {
    let fx = fixture().await;
    let id = oxplow_domain::uri::build::id_of(
        run(&fx, "triage.summary.ask", ask_as("item-p", "sort")).await["jobRun"]
            .as_str()
            .unwrap(),
        oxplow_domain::uri::resources::JOB_RUN,
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

/// Intent is durable: a job recorded before oxplow stopped runs after it
/// starts again.
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
    let id = oxplow_domain::uri::build::id_of(
        run_on(&before, "triage.summary.ask", ask("item-3")).await["jobRun"]
            .as_str()
            .unwrap(),
        oxplow_domain::uri::resources::JOB_RUN,
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
