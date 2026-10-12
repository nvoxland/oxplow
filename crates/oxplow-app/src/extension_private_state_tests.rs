//! End to end: extensions on disk declare commands over the private state
//! and configuration operations, and they run through the real command bus.

use std::path::Path;

use oxplow_db::extension_state_store::SqliteExtensionStateStore;
use oxplow_domain::extension_state::{Area, Namespace, Scope};
use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use crate::test_fixtures::{services_with_effort, EffortFixture};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

const HUMAN_ONLY: &str = "invokers: { human: true, agent: false, lens: false }";

/// An extension whose commands read and write its own private state and
/// configuration, plus a script that reads the state back.
fn declare(root: &Path, name: &str, configuration: &str) {
    let manifest = format!(
        "manifest: 2\nname: {name}\nintent:\n  purpose: p\n{configuration}commands:
  - name: state.write
    summary: Replace the private state.
    scope: state.write
    op: compare_exchange
    {HUMAN_ONLY}
  - name: state.read
    summary: Read the private state.
    scope: state.read
    op: snapshot
    {HUMAN_ONLY}
  - name: state.peek
    summary: Read the private state from a script.
    input_schema: {{ type: object }}
    entry: handlers/peek.star
    needs: [state.read]
    {HUMAN_ONLY}
  - name: config.read
    summary: The effective configuration.
    scope: extension_config.read
    op: snapshot
    {HUMAN_ONLY}
  - name: config.inspect
    summary: The layer stamps.
    scope: extension_config.read
    op: inspect
    {HUMAN_ONLY}
  - name: config.write
    summary: Replace configuration overrides.
    scope: extension_config.write
    op: compare_exchange
    {HUMAN_ONLY}
"
    );
    write(
        root,
        &format!("oxplow/extensions/{name}/extension.yaml"),
        &manifest,
    );
    write(
        root,
        &format!("oxplow/extensions/{name}/handlers/peek.star"),
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"state.read\", {\"scope\": \"project\", \"area\": \"state\"})}\n",
    );
}

const CONFIGURATION: &str = "configuration:
  version: 1
  schema:
    type: object
    required: [limit]
    additionalProperties: false
    properties:
      limit: { type: integer, minimum: 1, maximum: 100 }
      label: { type: string }
      nested:
        type: object
        properties:
          a: { type: integer }
          b: { type: integer }
  defaults:
    limit: 10
    label: default
    nested: { a: 1, b: 1 }
";

async fn run(fx: &EffortFixture, name: &str, input: Value) -> Result<Value, CommandError> {
    fx.svc
        .commands
        .run(&Actor::Human, name, input, false)
        .await
        .map(|outcome| outcome.result)
}

async fn reconcile(fx: &EffortFixture) {
    crate::test_fixtures::activate(&fx.svc).await;
}

fn write_state(scope: &str, expected: Value, values: Value) -> Value {
    json!({ "scope": scope, "area": "state", "expected": expected,
            "schema_version": 1, "values": values })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_writes_state_and_reads_it_back() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", "");
    reconcile(&fx).await;

    let absent = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(absent, Value::Null);

    let stamp = run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"cursor": {"page": 4}})),
    )
    .await
    .unwrap();
    assert_eq!(stamp, json!({"schema_version": 1, "revision": 1}));

    let read = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(
        read,
        json!({"stamp": {"schema_version": 1, "revision": 1}, "values": {"cursor": {"page": 4}}})
    );

    // A script declaring `state.read` sees the same snapshot.
    let peeked = run(&fx, "alpha.state.peek", json!({})).await.unwrap();
    assert_eq!(peeked["result"], read, "{peeked}");

    // The write is audited without its values.
    let audits = fx.svc.commands.audit_store().list_recent(20).await.unwrap();
    let audit = audits
        .iter()
        .find(|a| a.command == "alpha.state.write")
        .expect("the write is audited");
    assert!(
        !format!("{audit:?}").contains("cursor"),
        "private values stay out of the audit: {audit:?}"
    );

    // A stale stamp is refused through the bus, and nothing changes.
    let stale = run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"cursor": 9})),
    )
    .await;
    assert!(
        matches!(stale, Err(CommandError::Invalid { .. })),
        "{stale:?}"
    );
    let next = run(
        &fx,
        "alpha.state.write",
        write_state(
            "project",
            json!({"schema_version": 1, "revision": 1}),
            json!({"cursor": 5}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(next["revision"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_extension_cannot_read_the_first_ones_namespace() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", "");
    declare(fx._dir.path(), "beta", "");
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"secret-ish": "alpha"})),
    )
    .await
    .unwrap();

    // The same request, from beta, reads beta's own (empty) namespace.
    let beta = run(
        &fx,
        "beta.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(beta, Value::Null);

    // And a request cannot name another owner.
    let named = run(
        &fx,
        "beta.state.read",
        json!({"scope": "project", "area": "state", "extension": "alpha"}),
    )
    .await;
    assert!(
        matches!(named, Err(CommandError::Invalid { .. })),
        "{named:?}"
    );

    // beta's own write lands in beta's namespace only.
    run(
        &fx,
        "beta.state.write",
        write_state("project", Value::Null, json!({"mine": 1})),
    )
    .await
    .unwrap();
    let alpha = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(alpha["values"], json!({"secret-ish": "alpha"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_extension_is_denied_while_its_rows_are_kept() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"kept": true})),
    )
    .await
    .unwrap();
    run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 7}}}),
    )
    .await
    .unwrap();

    write(
        fx._dir.path(),
        ".oxplow/project.yaml",
        "extensions:\n  disabled: [alpha]\n",
    );
    reconcile(&fx).await;
    assert!(fx.svc.commands.spec("alpha.state.read").is_none());
    assert!(run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"})
    )
    .await
    .is_err());
    assert!(run(&fx, "alpha.config.read", json!({})).await.is_err());

    // The rows are still there for the extension's return.
    let rows = SqliteExtensionStateStore::new(fx.svc.db.clone())
        .snapshot(Namespace {
            extension: "alpha".into(),
            scope: Scope::Project,
            area: Area::State,
        })
        .await
        .unwrap()
        .expect("disabling keeps the state row");
    assert_eq!(rows.values["kept"], json!(true));

    std::fs::remove_file(fx._dir.path().join(".oxplow/project.yaml")).unwrap();
    reconcile(&fx).await;
    let read = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(read["values"], json!({"kept": true}));
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(7));
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_resolves_defaults_then_project_then_personal() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;

    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(
        config["values"],
        json!({"limit": 10, "label": "default", "nested": {"a": 1, "b": 1}})
    );
    assert_eq!(config["origins"]["/limit"], json!("defaults"));
    assert_eq!(config["project"], Value::Null);

    let stamps = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1,
               "project": {"expected": null, "values": {"limit": 20, "nested": {"a": 2}}},
               "personal": {"expected": null, "values": {"limit": 30}}}),
    )
    .await
    .unwrap();
    assert_eq!(
        stamps["project"],
        json!({"schema_version": 1, "revision": 1})
    );
    assert_eq!(
        stamps["personal"],
        json!({"schema_version": 1, "revision": 1})
    );

    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(
        config["values"],
        json!({"limit": 30, "label": "default", "nested": {"a": 2, "b": 1}})
    );
    assert_eq!(config["origins"]["/limit"], json!("personal"));
    assert_eq!(config["origins"]["/nested/a"], json!("project"));
    assert_eq!(config["origins"]["/nested/b"], json!("defaults"));
    assert_eq!(config["origins"]["/label"], json!("defaults"));

    // Resetting a layer is an empty object at the next revision.
    run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1,
               "personal": {"expected": {"schema_version": 1, "revision": 1}, "values": {}}}),
    )
    .await
    .unwrap();
    let config = run(&fx, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(20));
    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(
        inspected["personal"],
        json!({"schema_version": 1, "revision": 2})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_failing_the_schema_is_refused_without_echoing_it() {
    let fx = services_with_effort().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;

    for bad in [
        json!({"limit": "hunter2-not-a-number"}),
        json!({"limit": 1000}),
        json!({"unknown-setting-hunter2": 1}),
    ] {
        let refused = run(
            &fx,
            "alpha.config.write",
            json!({"schema_version": 1, "project": {"expected": null, "values": bad}}),
        )
        .await
        .expect_err("a schema violation is refused");
        let message = format!("{refused:?} {refused}");
        assert!(matches!(refused, CommandError::Invalid { .. }), "{message}");
        assert!(
            !message.contains("hunter2") && !message.contains("1000"),
            "{message}"
        );
    }
    // Nothing was stored.
    let inspected = run(&fx, "alpha.config.inspect", json!({})).await.unwrap();
    assert_eq!(inspected["project"], Value::Null);

    // A wrong schema version and a stale stamp are refused too.
    let wrong_version = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 2, "project": {"expected": null, "values": {"limit": 5}}}),
    )
    .await;
    assert!(matches!(wrong_version, Err(CommandError::Invalid { .. })));
    run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 5}}}),
    )
    .await
    .unwrap();
    let stale = run(
        &fx,
        "alpha.config.write",
        json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 6}}}),
    )
    .await;
    assert!(
        matches!(stale, Err(CommandError::Invalid { .. })),
        "{stale:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn state_and_configuration_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    declare(dir.path(), "alpha", CONFIGURATION);
    let db_path = dir.path().join("restart.sqlite");
    let boot = || async {
        let svc = crate::Services::on_machine_with_database(
            dir.path(),
            dir.path().join(".oxplow/global-config"),
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            crate::providers::host::process_env(),
            oxplow_db::Database::open(&db_path).unwrap(),
        )
        .await
        .unwrap();
        svc
    };

    let first = boot().await;
    crate::test_fixtures::activate(&first).await;
    for (name, input) in [
        (
            "alpha.state.write",
            write_state("personal", Value::Null, json!({"kept": [1, 2]})),
        ),
        (
            "alpha.config.write",
            json!({"schema_version": 1, "project": {"expected": null, "values": {"limit": 42}}}),
        ),
    ] {
        first
            .commands
            .run(&Actor::Human, name, input, false)
            .await
            .unwrap();
    }
    drop(first);

    // A new process over the same file: nothing in memory carries over.
    let second = boot().await;
    crate::test_fixtures::activate(&second).await;
    let read = second
        .commands
        .run(
            &Actor::Human,
            "alpha.state.read",
            json!({"scope": "personal", "area": "state"}),
            false,
        )
        .await
        .unwrap()
        .result;
    assert_eq!(read["values"], json!({"kept": [1, 2]}));
    assert_eq!(read["stamp"]["revision"], json!(1));
    let config = second
        .commands
        .run(&Actor::Human, "alpha.config.read", json!({}), false)
        .await
        .unwrap()
        .result;
    assert_eq!(config["values"]["limit"], json!(42));
    assert_eq!(config["project"]["revision"], json!(1));
}
