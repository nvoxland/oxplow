//! A job's model rounds streamed as its attempt's output, cancelled
//! mid-stream, and its model's tool calls run through the extension's own
//! commands (and `query_sql`), against the in-process fixture model.

use oxplow_ai::config::{ProviderConfig, Role, RoleBinding};
use oxplow_ai::fixture::{FixtureModel, FixtureReply, FixtureToolCall};
use oxplow_domain::Actor;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub(super) const MANIFEST: &str = r#"manifest: 2
name: triage
intent:
  purpose: p
permissions: [sql.read, ai.compute]
jobs:
  - id: review
    description: Review the TODOs.
    version: 1
    operation: ai.complete_typed
    needs: [ai.compute, sql.read]
    tools: [todos.lookup]
    max_tool_rounds: 3
    timeout: 5
commands:
  - { name: review.ask, summary: Ask for a review., scope: jobs.write, op: enqueue }
  - { name: review.cancel, summary: Stop it., scope: jobs.write, op: cancel }
  - { name: review.result, summary: Read it., scope: jobs.read, op: snapshot }
  - name: todos.lookup
    summary: The TODOs in one file.
    entry: commands/lookup.star
    input_schema: { type: object, required: [path], properties: { path: { type: string } }, additionalProperties: false }
    access: read
    job: true
"#;

/// [`MANIFEST`] with its job declared `replay: idempotent`.
pub(super) fn manifest_idempotent() -> String {
    MANIFEST.replace(
        "    timeout: 5\n",
        "    timeout: 5\n    replay: idempotent\n",
    )
}

pub(super) const LOOKUP: &str = r#"
def transform(x):
    return {"commands": [], "result": {"todos": ["fix the parser in " + x["input"]["path"]]}}
"#;

pub(super) struct Fixture {
    pub(super) _dir: tempfile::TempDir,
    pub(super) svc: Arc<crate::Services>,
    pub(super) model: Arc<FixtureModel>,
}

/// `svc` with the summarize role on a fixture model answering `replies` in
/// turn, the extensions it has deployed and its jobs.
pub(super) async fn prepare(
    svc: &Arc<crate::Services>,
    replies: Vec<FixtureReply>,
) -> Arc<FixtureModel> {
    let model = Arc::new(FixtureModel::new(replies));
    svc.ai.providers().register_host(model.clone());
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "fixture".into(),
                kind: oxplow_ai::fixture::KIND.into(),
                base_url: None,
            },
            None,
        )
        .unwrap();
    svc.ai
        .set_role(
            &Role::SUMMARIZE,
            Some(RoleBinding::new("fixture", "fixture")),
        )
        .unwrap();
    // The extension the test wrote is deployed.
    crate::test_fixtures::activate(svc).await;
    model
}

/// The `triage` extension with its `review` job, and the
/// summarize role on a fixture model answering `replies` in turn.
pub(super) async fn fixture(manifest: &str, replies: Vec<FixtureReply>) -> Fixture {
    let fx = fixture_idle(manifest, replies).await;
    fx.svc
        .jobs
        .spawn(fx.svc.extension_activation.clone(), fx.svc.commands.clone());
    fx
}

/// [`fixture`] with the scheduler not started: jobs stay as they are
/// enqueued.
pub(super) async fn fixture_idle(manifest: &str, replies: Vec<FixtureReply>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/triage");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(ext.join("extension.yaml"), manifest).unwrap();
    std::fs::write(ext.join("commands/lookup.star"), LOOKUP).unwrap();
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    let model = prepare(&svc, replies).await;
    Fixture {
        _dir: dir,
        svc,
        model,
    }
}

pub(super) fn reply(text: &str) -> FixtureReply {
    text.into()
}

pub(super) fn slow(text: &str, every_ms: u64) -> FixtureReply {
    FixtureReply {
        text: text.into(),
        chunk_delay: Duration::from_millis(every_ms),
        ..FixtureReply::default()
    }
}

pub(super) fn calls(calls: &[(&str, Value)]) -> FixtureReply {
    FixtureReply {
        tool_calls: calls
            .iter()
            .map(|(name, arguments)| FixtureToolCall {
                name: (*name).into(),
                arguments: arguments.clone(),
            })
            .collect(),
        ..FixtureReply::default()
    }
}

pub(super) async fn run(fx: &Fixture, actor: &Actor, command: &str, input: Value) -> Value {
    fx.svc
        .commands
        .run(actor, command, input, false)
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"))
        .result
}

pub(super) async fn ask(fx: &Fixture, intent: &str) -> String {
    let queued = run(
        fx,
        &Actor::Human,
        "triage.review.ask",
        json!({
            "job": "review",
            "intentKey": intent,
            "request": {
                "schemaVersion": 1,
                "role": "summarize",
                "messages": [{"role": "user", "parts": [{"type": "text", "text": "Review the TODOs."}]}],
            },
        }),
    )
    .await;
    oxplow_domain::uri::build::id_of(
        queued["jobRun"].as_str().unwrap(),
        oxplow_domain::uri::resources::JOB_RUN,
    )
    .unwrap()
    .to_owned()
}

pub(super) async fn until(fx: &Fixture, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = run(
            fx,
            &Actor::Human,
            "triage.review.result",
            json!({ "job_run": id.to_string() }),
        )
        .await;
        if done(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the job never got there: {snapshot}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(super) async fn rows(fx: &Fixture, sql: &str, params: Vec<Value>) -> Vec<Vec<Value>> {
    let out = fx
        .svc
        .sql
        .query_sql(sql, params.into_iter().map(Into::into).collect(), None)
        .await
        .unwrap();
    serde_json::from_value(serde_json::to_value(out.rows).unwrap()).unwrap()
}

async fn page(fx: &Fixture, id: &str, params: Vec<(&str, oxplow_db::SqlCell)>) -> Value {
    let params: Vec<(String, oxplow_db::SqlCell)> = params
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let run = crate::test_fixtures::run_as_page(&fx.svc, id, &params).await;
    serde_json::to_value(&run.rows).unwrap()
}

pub(super) async fn completions(fx: &Fixture, id: &str) -> Vec<String> {
    rows(
        fx,
        "SELECT json_extract(payload, '$.outcome') FROM v_event
          WHERE type = 'contribution.job.completed' AND json_extract(payload, '$.job_run') = ?1",
        vec![json!(id)],
    )
    .await
    .into_iter()
    .map(|row| row[0].as_str().unwrap_or_default().to_string())
    .collect()
}

/// A streamed reply is the attempt's output while it comes: text rows
/// appear before the job ends, `v_job_run` counts its round, and the result
/// is the whole answer. The job keeps what it was about, its intent key.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_streams_its_reply_as_output_rows() {
    let fx = fixture(MANIFEST, vec![slow("one two three four five six", 60)]).await;
    let id = ask(&fx, "stream").await;

    let partway = until(&fx, &id, |s| {
        s["state"] == json!("running") && !s["text"].as_str().unwrap_or_default().is_empty()
    })
    .await;
    assert_ne!(
        partway["text"],
        json!("one two three four five six"),
        "{partway}"
    );

    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["text"], json!("one two three four five six"), "{done}");
    assert_eq!(done["intentKey"], json!("stream"), "{done}");
    assert_eq!(
        done["result"]["parts"],
        json!([{ "type": "text", "text": "one two three four five six" }])
    );
    let text_rows = rows(
        &fx,
        "SELECT count(*) FROM v_job_run_output WHERE job_id = ?1 AND kind = 'text'",
        vec![json!(id)],
    )
    .await;
    assert!(text_rows[0][0].as_i64().unwrap() > 1, "streamed in pieces");
    let job = rows(
        &fx,
        "SELECT state, ai_calls, extension, job, intent_key FROM v_job_run WHERE id = ?1",
        vec![json!(id)],
    )
    .await;
    assert_eq!(
        job[0],
        [
            json!("succeeded"),
            json!(1),
            json!("triage"),
            json!("oxplow:oxplow/job?job=triage/review"),
            json!("stream")
        ]
    );
    assert_eq!(completions(&fx, &id).await, ["succeeded"]);
}

/// Cancelling a running job stops its model mid-reply: it ends
/// `cancelled` with what had come, its call is recorded as cancelled with
/// that partial reply, and its completion says so.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_running_job_stops_its_stream() {
    let words = (0..200)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fx = fixture(MANIFEST, vec![slow(&words, 20)]).await;
    let id = ask(&fx, "cancel").await;
    until(&fx, &id, |s| {
        !s["text"].as_str().unwrap_or_default().is_empty()
    })
    .await;

    run(
        &fx,
        &Actor::Human,
        "triage.review.cancel",
        json!({ "job_run": &id.to_string() }),
    )
    .await;
    let done = until(&fx, &id, |s| s["state"] == json!("cancelled")).await;
    let text = done["result"]["parts"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.starts_with("w0 "), "the partial reply is kept: {done}");
    assert!(text.len() < words.len(), "it stopped early: {done}");
    assert_eq!(done["result"]["stopped"], json!("cancelled"));

    let call = rows(
        &fx,
        "SELECT ok, error FROM v_ai_call WHERE caller LIKE 'oxplow:oxplow/job_run?%' ORDER BY id DESC LIMIT 1",
        vec![],
    )
    .await;
    assert_eq!(call[0][0], json!(0));
    assert!(
        call[0][1].as_str().unwrap().contains("cancelled"),
        "{call:?}"
    );
    assert_eq!(completions(&fx, &id).await, ["cancelled"]);
}

/// The model's tool call runs the extension's command under the job, the
/// answer goes back to the model, and its next round is the result. The
/// call, its answer and both rounds are the attempt's output.
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_call_runs_the_extensions_command_and_answers_the_model() {
    let fx = fixture(
        MANIFEST,
        vec![
            calls(&[("todos_lookup", json!({ "path": "src/lib.rs" }))]),
            reply("One parser TODO."),
        ],
    )
    .await;
    let id = ask(&fx, "tools").await;
    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["result"]["rounds"], json!(2), "{done}");
    assert_eq!(done["toolCalls"], json!(1));

    // The model was told its tools, then given the command's answer.
    let asked = fx.model.asked_typed();
    let names: Vec<_> = asked[0].tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["todos_lookup", "query_sql"]);
    let answer = serde_json::to_value(&asked[1].messages.last().unwrap().parts).unwrap();
    assert_eq!(
        answer[0]["result"]["todos"],
        json!(["fix the parser in src/lib.rs"]),
        "{answer}"
    );

    let kinds: Vec<String> = rows(
        &fx,
        "SELECT kind FROM v_job_run_output WHERE job_id = ?1 ORDER BY seq",
        vec![json!(id)],
    )
    .await
    .into_iter()
    .map(|r| r[0].as_str().unwrap().to_string())
    .collect();
    assert_eq!(
        kinds,
        ["tool_call", "usage", "tool_result", "text", "usage"]
    );

    // The command ran as the job: its run says so, and whose run it was
    // nested in.
    let ran = rows(
        &fx,
        "SELECT actor_kind, actor, parent_run_id IS NOT NULL FROM v_extension_run
          WHERE contribution = 'oxplow:oxplow/command?command=triage.todos.lookup'",
        vec![],
    )
    .await;
    assert_eq!(ran.len(), 1, "{ran:?}");
    assert_eq!(ran[0][0], json!("job"));
    assert_eq!(
        ran[0][1],
        json!(oxplow_domain::uri::build::job_run_uri(&id)),
        "the job run is the actor"
    );

    // oxplow's own Job Runs page lists it, and its output page follows it.
    let run_ref = oxplow_domain::uri::build::job_run_uri(&id);
    let jobs = page(&fx, "oxplow-bundled/job-runs", vec![]).await;
    assert_eq!(jobs[0][0], json!(run_ref), "{jobs}");
    assert_eq!(jobs[0][2], json!("oxplow:oxplow/job?job=triage/review"));
    assert_eq!(jobs[0][3], json!("tools"), "what it was about");
    assert_eq!(jobs[0][4], json!("succeeded"));
    let output = page(
        &fx,
        "oxplow-bundled/job-run-output",
        vec![("job_run", oxplow_db::SqlCell::Text(id.to_string()))],
    )
    .await;
    let content: Vec<_> = output
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[4].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        content[0], r#"todos_lookup {"path":"src/lib.rs"}"#,
        "{output}"
    );
    assert_eq!(content[3], "One parser TODO.");
}

/// A tool call the command refuses, one with input its schema doesn't
/// take, and one naming no tool are each answered with why — the job goes
/// on; past its rounds it fails.
#[tokio::test(flavor = "multi_thread")]
async fn bad_tool_calls_are_answered_and_rounds_run_out() {
    let fx = fixture(
        MANIFEST,
        vec![calls(&[
            ("todos_lookup", json!({ "file": "x" })),
            ("rm_rf", json!({})),
        ])],
    )
    .await;
    let id = ask(&fx, "bad").await;
    let done = until(&fx, &id, |s| s["state"] == json!("failed")).await;
    assert_eq!(
        done["result"]["error"],
        json!("tool rounds exhausted"),
        "{done}"
    );
    assert_eq!(done["result"]["rounds"], json!(3));
    let errors = rows(
        &fx,
        "SELECT json_extract(data, '$.error') FROM v_job_run_output
          WHERE job_id = ?1 AND kind = 'tool_result' ORDER BY seq LIMIT 2",
        vec![json!(id)],
    )
    .await;
    assert!(
        errors[0][0].as_str().unwrap().contains("path"),
        "the schema's complaint: {errors:?}"
    );
    assert_eq!(errors[1][0], json!("no tool `rm_rf`"));
}

/// With `sql.read` the model may query the published models.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_reads_sql_answers_query_sql() {
    let fx = fixture(
        MANIFEST,
        vec![
            calls(&[(
                "query_sql",
                json!({ "sql": "SELECT job FROM v_job_run WHERE state = ?1", "params": ["running"] }),
            )]),
            reply("done"),
        ],
    )
    .await;
    let id = ask(&fx, "sql").await;
    until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    let asked = fx.model.asked_typed();
    let answer = serde_json::to_value(&asked[1].messages.last().unwrap().parts).unwrap();
    assert_eq!(
        answer[0]["result"]["rows"],
        json!([["oxplow:oxplow/job?job=triage/review"]]),
        "{answer}"
    );
}

/// A job that outlives its `timeout` fails, keeping what it had streamed.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_past_its_timeout_fails() {
    let manifest = MANIFEST.replace("timeout: 5", "timeout: 1");
    let words = (0..100)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fx = fixture(&manifest, vec![slow(&words, 50)]).await;
    let id = ask(&fx, "slow").await;
    let done = until(&fx, &id, |s| s["state"] == json!("failed")).await;
    assert_eq!(
        done["result"]["error"],
        json!("timed out after 1s"),
        "{done}"
    );
    assert!(!done["text"].as_str().unwrap().is_empty());
    assert_eq!(completions(&fx, &id).await, ["failed"]);
}

/// A person stops any job (`oxplow.job.cancel`), whichever extension
/// started it; a queued one ends at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_cancels_any_job() {
    let words = (0..200)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fx = fixture(MANIFEST, vec![slow(&words, 20)]).await;
    let id = ask(&fx, "person").await;
    until(&fx, &id, |s| s["state"] == json!("running")).await;
    run(
        &fx,
        &Actor::Human,
        "oxplow.job_run.cancel",
        json!({ "job_run": &id.to_string() }),
    )
    .await;
    until(&fx, &id, |s| s["state"] == json!("cancelled")).await;
    let refused = fx
        .svc
        .commands
        .run(
            &Actor::Agent {
                session_id: None,
                thread_id: None,
                stream_id: None,
            },
            "oxplow.job_run.cancel",
            json!({ "job_run": &id.to_string() }),
            false,
        )
        .await;
    assert!(refused.is_err(), "an agent doesn't stop a person's jobs");
}

/// A tool must be one of the extension's commands that admits a job.
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_that_does_not_admit_a_job_is_a_load_error() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/triage");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(ext.join("commands/lookup.star"), LOOKUP).unwrap();
    for (manifest, says) in [
        (
            MANIFEST.replace("    job: true\n", ""),
            "doesn't admit a job",
        ),
        (
            MANIFEST.replace("    access: read\n", ""),
            "a tool only reads",
        ),
        (
            MANIFEST.replace("tools: [todos.lookup]", "tools: [todos.find]"),
            "none of this extension's commands",
        ),
        (
            MANIFEST.replace(
                "needs: [ai.compute, sql.read]",
                "needs: [ai.compute, tree.read]",
            ),
            "job needs are",
        ),
    ] {
        std::fs::write(ext.join("extension.yaml"), &manifest).unwrap();
        let errors =
            crate::extension::extensions::load_project_extension(dir.path(), "triage").errors;
        assert!(
            errors.iter().any(|e| e.contains(says)),
            "{says}: {errors:?}"
        );
    }
}
