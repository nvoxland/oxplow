//! A contribution's run as `v_extension_run` shows it: who it was, what it
//! was caused by, how it ended, and the data it affected — for a run that
//! succeeded, failed and was tried again, was cancelled, and was refused —
//! without a secret value, a prompt or a path anywhere in the row.

use std::os::unix::fs::PermissionsExt;

use oxplow_domain::run::{Mode, RunContext};
use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::collector_runner::{run_collector, Collectors, RunTrigger};
use crate::test_fixtures::{services_with_task_effort, TaskEffortFixture};

const SECRET: &str = "s3cr3t-token-value";

const MANIFEST: &str = "manifest: 2\nname: acme\nversion: 2.1.0\nintent:\n  purpose: p\npermissions: { process: true, secrets: [GH_TOKEN] }\ncollectors:
  - id: sync
    runtime: exec
    entry: sync.sh
    credentials: [GH_TOKEN]
    entities:
      - { name: seen, key: k, columns: { k: text } }
commands:
  - name: human.only
    summary: Only a person.
    input_schema: { type: object }
    entry: only.star
    invokers: { human: true, agent: false, lens: false }
";

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(".oxplow/extensions/acme").join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    if rel.ends_with(".sh") {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Install `acme` with the program `body` and set its credential.
async fn install(fx: &TaskEffortFixture, body: &str) {
    let root = fx.svc.layout.project_dir.clone();
    write(&root, "extension.yaml", MANIFEST);
    write(&root, "sync.sh", body);
    write(
        &root,
        "only.star",
        "def transform(x):\n    return {\"commands\": [], \"result\": {}}\n",
    );
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let collectors = Collectors::of(&fx.svc, &root);
    crate::collector_runner::set_credential(&collectors, "acme", "GH_TOKEN", Some(SECRET)).unwrap();
}

async fn run(fx: &TaskEffortFixture, parent: &RunContext) -> Result<(), String> {
    let root = fx.svc.layout.project_dir.clone();
    run_collector(
        &Collectors::of(&fx.svc, &root),
        parent,
        "acme",
        "sync",
        RunTrigger::Manual,
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("{e:?}"))
}

fn person() -> RunContext {
    RunContext::core(Actor::Human, Mode::External)
}

async fn rows(fx: &TaskEffortFixture, sql: &str) -> Vec<Vec<Value>> {
    let out = fx.svc.sql.query_sql(sql, vec![], None).await.unwrap();
    serde_json::to_value(&out.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().clone())
        .collect()
}

/// Whether the collector's data is current, and its state.
async fn fresh(fx: &TaskEffortFixture) -> Vec<Vec<Value>> {
    rows(
        fx,
        "SELECT fresh, state FROM v_contribution_health WHERE ref = 'oxplow:oxplow/collector?collector=acme/sync'",
    )
    .await
}

const RUNS: &str = "SELECT outcome, error, version, generation, actor, latest, mode, run_id
                      FROM v_extension_run WHERE contribution = 'oxplow:oxplow/collector?collector=acme/sync'
                     ORDER BY started_at, run_id";

/// A run that failed and one that came after it: each is its own row, with
/// the extension's version, the actor and the failure; only the last is
/// `latest`; the collector's own row names the last, and no secret or
/// program output beyond the failure's reason is in any column.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_and_its_retry_are_two_rows_and_neither_leaks_a_credential() {
    let fx = services_with_task_effort().await;
    let failing = "#!/bin/sh\necho \"cannot reach host with token $GH_TOKEN\" >&2\nexit 3\n";
    install(&fx, failing).await;
    let failure = run(&fx, &person()).await.expect_err("it fails");
    assert!(!failure.contains(SECRET), "{failure}");
    // Its data is the last good run's, so after a failure it isn't fresh.
    assert_eq!(fresh(&fx).await, vec![vec![json!(0), json!("failing")]]);

    write(
        &fx.svc.layout.project_dir,
        "sync.sh",
        "#!/bin/sh\necho '{\"entities\":{\"seen\":[{\"k\":\"a\"}]}}'\n",
    );
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    run(&fx, &person()).await.expect("the retry succeeds");
    assert_eq!(fresh(&fx).await, vec![vec![json!(1), json!("ok")]]);

    let runs = rows(&fx, RUNS).await;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(runs[0][0], json!("failed"));
    assert_eq!(runs[1][0], json!("ok"));
    assert_eq!(runs[0][2], json!("2.1.0"), "its version");
    // A collector's run belongs to its extension's version and holds no
    // catalog generation (a command's run records the one it pinned).
    assert_eq!(runs[0][3], Value::Null, "{:?}", runs[0]);
    assert_eq!(runs[0][4], json!("human"), "who");
    assert_eq!(
        (runs[0][5].clone(), runs[1][5].clone()),
        (json!(0), json!(1))
    );

    // The collector's row points at the run that last touched its data; the
    // failed run's attempt stays recorded under its own id.
    let last = rows(
        &fx,
        "SELECT run_id, status FROM v_collector_run WHERE owner = 'acme' AND id = 'sync'",
    )
    .await;
    assert_eq!(last, vec![vec![runs[1][7].clone(), json!("ok")]]);

    // Nothing in any recorded column carries the credential or the program's
    // own output.
    let everything = rows(
        &fx,
        "SELECT json_group_array(json_object('e', error, 'd', decisions, 's', scopes, 'a', actor))
           FROM v_extension_run",
    )
    .await;
    assert!(
        !everything[0][0].to_string().contains(SECRET),
        "{everything:?}"
    );
}

/// A run cancelled mid-way is `cancelled`, not a failure that counts toward
/// disabling the collector, and it records no rows.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_run_is_recorded_as_cancelled_and_publishes_nothing() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    let started = root.join("started");
    install(
        &fx,
        &format!(
            "#!/bin/sh\ntouch {}\nsleep 30\necho '{{\"entities\":{{\"seen\":[{{\"k\":\"a\"}}]}}}}'\n",
            started.display()
        ),
    )
    .await;
    let parent = person();
    let cancel = parent.cancel().clone();
    let running = {
        let fx_run = parent.clone();
        let svc = fx.svc.clone();
        let root = root.clone();
        tokio::spawn(async move {
            run_collector(
                &Collectors::of(&svc, &root),
                &fx_run,
                "acme",
                "sync",
                RunTrigger::Manual,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !started.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the program started");
    cancel.cancel();
    running.await.unwrap().expect_err("it stops");

    let runs = rows(&fx, RUNS).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0][0], json!("cancelled"), "{runs:?}");
    let seen = rows(&fx, "SELECT count(*) FROM v_acme_seen").await;
    assert_eq!(seen, vec![vec![json!(0)]]);
    let health = rows(
        &fx,
        "SELECT consecutive_failures FROM v_contribution_health WHERE ref = 'oxplow:oxplow/collector?collector=acme/sync'",
    )
    .await;
    assert!(
        health.is_empty() || health == vec![vec![json!(0)]],
        "a cancellation isn't a failure of the collector: {health:?}"
    );
}

/// A caller the command doesn't admit is refused, and the refusal is a
/// `denied` run of that command — not a failure — owned by the extension.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_caller_is_a_denied_run_not_a_failure() {
    let fx = services_with_task_effort().await;
    install(&fx, "#!/bin/sh\necho '{}'\n").await;
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    fx.svc
        .commands
        .run(&agent, "acme.human.only", json!({}), false)
        .await
        .expect_err("an agent may not run it");
    let runs = rows(
        &fx,
        "SELECT outcome, version, kind, error FROM v_extension_run
          WHERE contribution = 'oxplow:oxplow/command?command=acme.human.only'",
    )
    .await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0][0], json!("denied"));
    assert_eq!(runs[0][1], json!("2.1.0"));
    assert_eq!(runs[0][2], json!("command"));
}

/// A revocation reaches a Starlark collector that is computing with no host
/// call to notice it by: the run ends within a moment, recorded `cancelled`,
/// not when its timeout does.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_an_extension_stops_a_collector_script_that_is_computing() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        "extension.yaml",
        "manifest: 2\nname: acme\nversion: 2.1.0\nintent:\n  purpose: p\ncollectors:
  - id: spin
    runtime: starlark
    entry: spin.star
    entities:
      - { name: seen, key: k, columns: { k: text } }
",
    );
    write(
        &root,
        "spin.star",
        "def transform(x):\n    n = 0\n    for i in range(100000):\n        for j in range(100000):\n            n += 1\n    return {\"entities\": {\"seen\": []}}\n",
    );
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let running = {
        let (svc, root) = (fx.svc.clone(), root.clone());
        tokio::spawn(async move {
            run_collector(
                &Collectors::of(&svc, &root),
                &person(),
                "acme",
                "spin",
                RunTrigger::Manual,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    std::fs::write(
        root.join(".oxplow/personal.yaml"),
        "disableExtensions: [acme]\n",
    )
    .unwrap();
    fx.svc.reload_config_from_disk().unwrap();
    let disabled = std::time::Instant::now();
    running.await.unwrap().expect_err("it is stopped");
    assert!(
        disabled.elapsed() < std::time::Duration::from_secs(2),
        "it ran on for {:?} after the extension was disabled",
        disabled.elapsed()
    );
    let runs = rows(
        &fx,
        "SELECT outcome FROM v_extension_run WHERE contribution = 'oxplow:oxplow/collector?collector=acme/spin'",
    )
    .await;
    assert_eq!(runs, vec![vec![json!("cancelled")]]);
}
